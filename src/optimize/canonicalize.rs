use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{BinOp, BlockId, Function, Global, GlobalId, Instr, LocalId, OpKind, Operand};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::{Ty, TyRegistry};
use indexmap::{IndexMap, IndexSet};
use slotmap::{SecondaryMap, SlotMap};
use tracing::instrument;

/// Canonicalize
///
/// Folds the constants in chains of binary instructions, e.g. `((n + 3) + m) + 5` becomes
/// `(n + m) + 8`.
///
/// For `t1 = a OP1 c1` and `t2 = t1 OP2 x`, where `c1` is a constant and `OP1` and `OP2`
/// right-commute (see `OpKind::right_commutes_with`):
/// - **Fold:** if `x` is a constant, `t2` is rewritten to `a OP1 c` with `c` computed at compile
///   time, e.g. `(n - 3) - 5` becomes `n - 8`. `t1` is unchanged, and becomes dead if `t2` was
///   its only use.
/// - **Swap:** otherwise the pair is rewritten to `t1 = a OP2 x` and `t2 = t1 OP1 c1`, moving the
///   constant outward where a later fold can reach it. Each opcode keeps its own right operand,
///   so `(n - 3) + m` becomes `(n + m) - 3`. This requires that `t1` has exactly one use (`t2`),
///   since its value changes, that `x`'s definition dominates `t1`'s, since that is where
///   `x` is now read, and that `t1` is not live at any `Breakpoint`, since a breakpoint would
///   observe the changed value.
///
/// If `OP2` is commutative, `t1` may instead be its right operand (`t2 = x OP2 t1`).
/// Instructions are never inserted, removed, or moved; only operands and opcodes change. A
/// constant is a literal operand, or a local defined by a `Copy` of a literal.
pub struct Canonicalize;

impl Optimize for Canonicalize {
    #[instrument(name = "canonicalize", skip_all, fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(
        &self,
        function: &mut Function,
        _globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
    ) -> Outcome {
        // Rewrites never change the CFG or where any local is defined, so the dominators stay
        // valid for the whole pass. Folding can only remove uses, so stale use counts are
        // conservative.
        let use_counts = function.count_uses();
        let dominators = Dominators::new(function);
        let mut outcome = Outcome::Unmodified;
        for block_id in function.blocks.keys().collect::<Vec<_>>() {
            for outer_index in 0..function.blocks[block_id].instructions.len() {
                for rewrite in find_rewrites(
                    function,
                    ty_registry,
                    &dominators,
                    block_id,
                    outer_index,
                    &use_counts,
                ) {
                    rewrite.apply(function);
                    outcome = Outcome::Modified;
                }
            }
        }
        outcome
    }
}

fn defined_at(function: &Function, local: LocalId) -> Option<(BlockId, usize)> {
    function
        .locals
        .get(local)
        .and_then(|l| l.defined.iter().next())
        .map(|(block, &index)| (block, index))
}

/// The compile-time constant value of `operand`, if it has one: either a literal embedded directly
/// in the instruction, or a local whose sole definition is a `Copy` of a literal. The analyzer
/// always lowers a source-level literal used as a binary operand into the latter shape (`#n = copy
/// 5; x = add y, #n`) rather than embedding it directly, so matching only `Operand::Literal` would
/// never fire on freshly analyzed MIR.
fn constant(
    function: &Function,
    operand: &Operand,
    ty: Ty,
    ty_registry: &TyRegistry,
) -> Option<Value> {
    let literal = match operand {
        Operand::Literal(literal) => literal,
        Operand::Local(local) => {
            let (block, index) = defined_at(function, *local)?;
            match &function.blocks[block].instructions[index] {
                Instr::Copy {
                    source: Operand::Literal(literal),
                    ..
                } => literal,
                _ => return None,
            }
        }
        Operand::FnPtr(_) => return None,
    };
    let bytes = ty.bit_size(ty_registry).padded_bytes() as usize;
    Some(Value::from_bytes(
        ty,
        ty_registry,
        &literal.value.to_le_bytes()[..bytes],
    ))
}

fn to_literal(value: Value) -> Operand {
    let mut bytes = value.into_vec();
    bytes.resize(4, 0);
    Operand::Literal(IntegerLiteral::from(u32::from_le_bytes(
        *bytes.as_array::<4>().unwrap(),
    )))
}

/// Folds the constants of `(a op1 c1) op2 c2` into the single constant `c` such that it equals
/// `a op1 c`. Only defined when `op1` right-commutes with `op2`.
fn fold_constants(op1: OpKind, op2: OpKind, mut c1: Value, c2: Value) -> Value {
    match (op1, op2) {
        (OpKind::Add, OpKind::Add) | (OpKind::Sub, OpKind::Sub) => c1 += c2,
        (OpKind::Add, OpKind::Sub) | (OpKind::Sub, OpKind::Add) => c1 -= c2,
        (OpKind::BitAnd, OpKind::BitAnd) => c1 &= c2,
        (OpKind::BitOr, OpKind::BitOr) => c1 |= c2,
        (OpKind::BitXor, OpKind::BitXor) => c1 ^= c2,
        (op1, op2) => unreachable!("{op1:?} does not right-commute with {op2:?}"),
    }
    c1
}

/// Instruction-level dominance, derived from the function's block-level immediate dominators.
struct Dominators(IndexMap<Node, Node>);

impl Dominators {
    fn new(function: &Function) -> Self {
        Dominators(Graph::from(function).calculate_immediate_dominators())
    }

    /// Whether the instruction at `def` dominates the instruction at `at`: every path from the
    /// function's entry to `at` passes through `def` first.
    fn dominates(&self, def: (BlockId, usize), at: (BlockId, usize)) -> bool {
        let ((def_block, def_index), (at_block, at_index)) = (def, at);
        if def_block == at_block {
            return def_index < at_index;
        }
        let mut node = Node::from(at_block);
        // The entry block has no immediate dominator, and neither does an unreachable block.
        while let Some(&immediate_dominator) = self.0.get(&node) {
            if immediate_dominator == Node::from(def_block) {
                return true;
            }
            node = immediate_dominator;
        }
        false
    }
}

/// Whether the local defined at `def` and used only at `use_` is live at a
/// [`Instr::Breakpoint`], i.e. whether a breakpoint lies on some path from `def` to `use_`.
///
/// `def` must dominate `use_`, and `use_` must not be a `Phi`. Its only use then makes the live
/// range exactly the instructions reachable by walking backward from `use_` until `def`.
fn live_at_breakpoint(function: &Function, def: (BlockId, usize), use_: (BlockId, usize)) -> bool {
    let has_breakpoint =
        |instructions: &[Instr]| instructions.iter().any(|i| matches!(i, Instr::Breakpoint));
    let ((def_block, def_index), (use_block, use_index)) = (def, use_);
    let instructions = &function.blocks[use_block].instructions;
    if def_block == use_block {
        return has_breakpoint(&instructions[def_index + 1..use_index]);
    }
    if has_breakpoint(&instructions[..use_index]) {
        return true;
    }
    let mut visited = IndexSet::new();
    let mut worklist = function.blocks[use_block]
        .previous_blocks()
        .collect::<Vec<_>>();
    while let Some(block_id) = worklist.pop() {
        if !visited.insert(block_id) {
            continue;
        }
        let instructions = &function.blocks[block_id].instructions;
        if block_id == def_block {
            if has_breakpoint(&instructions[def_index + 1..]) {
                return true;
            }
            continue;
        }
        // This includes `use_`'s own block when reached again around a loop: the value stays
        // live after `use_` since the next iteration reads it again.
        if has_breakpoint(instructions) {
            return true;
        }
        worklist.extend(function.blocks[block_id].previous_blocks());
    }
    false
}

/// Replacement instructions, each at the `(block, index)` of the instruction it replaces.
struct Rewrite {
    position: (BlockId, usize),
    instr: Instr,
}

impl Rewrite {
    fn apply(self, function: &mut Function) {
        let (block, index) = self.position;
        function.blocks[block].instructions[index] = self.instr;
    }
}

/// Tries to find a rewrite for the outer instruction at `(block_id, outer_index)`: a local
/// operand of that instruction whose defining instruction is a right-commuting binary op holding
/// a constant. If the outer instruction's other operand is also a constant the two are folded,
/// otherwise the other operand is swapped into the defining instruction's position.
fn find_rewrites(
    function: &Function,
    ty_registry: &TyRegistry,
    dominators: &Dominators,
    block_id: BlockId,
    outer_index: usize,
    use_counts: &SecondaryMap<LocalId, u32>,
) -> Vec<Rewrite> {
    let outer_position = (block_id, outer_index);
    let Some(outer) = function.blocks[block_id].instructions[outer_index].as_binop() else {
        return vec![];
    };
    let ty = function.locals[outer.target].ty;

    // `left` is always eligible as the chain link; `right` only when `op` is commutative.
    let mut candidates = vec![(outer.left, outer.right.clone())];
    if outer.op.is_commutative()
        && let Operand::Local(right) = outer.right
    {
        candidates.push((right, Operand::Local(outer.left)));
    }

    for (link, other) in candidates {
        // SSA guarantees `link`'s definition dominates its use by the outer instruction, wherever
        // in the function that definition is.
        let Some(inner_position) = defined_at(function, link) else {
            continue;
        };
        let Some(inner) =
            function.blocks[inner_position.0].instructions[inner_position.1].as_binop()
        else {
            continue;
        };
        if !inner.op.right_commutes_with(outer.op) {
            continue;
        }
        let Some(c1) = constant(function, &inner.right, ty, ty_registry) else {
            continue;
        };

        if let Some(c2) = constant(function, &other, ty, ty_registry) {
            // `(a OP1 c1) OP2 c2` => `a OP1 (c1 ⊕ c2)`. Only the outer instruction changes, so
            // other uses of `t1` are unaffected.
            let folded = fold_constants(inner.op, outer.op, c1, c2);
            return vec![Rewrite {
                position: outer_position,
                instr: BinOp {
                    op: inner.op,
                    target: outer.target,
                    left: inner.left,
                    right: to_literal(folded),
                }
                .into_instr(),
            }];
        }

        // `(a OP1 c) OP2 x` => `(a OP2 x) OP1 c`. `x` moves to the inner instruction's position,
        // so its definition must dominate that position too, not just the outer instruction's.
        let Operand::Local(other) = other else {
            continue;
        };
        match defined_at(function, other) {
            Some(other_defined) if dominators.dominates(other_defined, inner_position) => {}
            _ => continue,
        }
        // `t1`'s value changes, so the outer instruction must be its only use.
        if use_counts.get(inner.target).copied().unwrap_or(0) != 1 {
            continue;
        }
        // A breakpoint observes every live value, so `t1` must not be live at one.
        if live_at_breakpoint(function, inner_position, outer_position) {
            continue;
        }
        return vec![
            Rewrite {
                position: inner_position,
                instr: BinOp {
                    op: outer.op,
                    target: inner.target,
                    left: inner.left,
                    right: Operand::Local(other),
                }
                .into_instr(),
            },
            Rewrite {
                position: outer_position,
                instr: BinOp {
                    op: inner.op,
                    target: outer.target,
                    left: inner.target,
                    right: inner.right,
                }
                .into_instr(),
            },
        ];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimize::graph::graph_test_support::loop_with_exit;

    /// Appends `instr` to `block`, returning its position.
    fn push(function: &mut Function, block: BlockId, instr: Instr) -> (BlockId, usize) {
        let instructions = &mut function.blocks[block].instructions;
        instructions.push(instr);
        (block, instructions.len() - 1)
    }

    // The analyzer routes every loop-carried value through a `copy` in the loop header, so a
    // definition outside a loop never feeds a binop inside it directly and these shapes can't yet
    // be reached from source.

    #[test]
    fn breakpoint_after_the_use_in_a_loop_is_live() {
        let (mut function, [b1, b2, _, _]) = loop_with_exit();
        let def = push(&mut function, b1, Instr::Nop);
        let use_ = push(&mut function, b2, Instr::Nop);
        push(&mut function, b2, Instr::Breakpoint);
        assert!(live_at_breakpoint(&function, def, use_));
    }

    #[test]
    fn breakpoint_on_the_back_edge_is_live() {
        let (mut function, [b1, b2, b3, _]) = loop_with_exit();
        let def = push(&mut function, b1, Instr::Nop);
        let use_ = push(&mut function, b2, Instr::Nop);
        push(&mut function, b3, Instr::Breakpoint);
        assert!(live_at_breakpoint(&function, def, use_));
    }

    #[test]
    fn breakpoint_on_the_loop_exit_is_not_live() {
        let (mut function, [b1, b2, _, b4]) = loop_with_exit();
        let def = push(&mut function, b1, Instr::Nop);
        let use_ = push(&mut function, b2, Instr::Nop);
        push(&mut function, b4, Instr::Breakpoint);
        assert!(!live_at_breakpoint(&function, def, use_));
    }

    #[test]
    fn breakpoint_before_the_def_is_not_live() {
        let (mut function, [b1, b2, _, _]) = loop_with_exit();
        push(&mut function, b1, Instr::Breakpoint);
        let def = push(&mut function, b1, Instr::Nop);
        let use_ = push(&mut function, b2, Instr::Nop);
        assert!(!live_at_breakpoint(&function, def, use_));
    }
}
