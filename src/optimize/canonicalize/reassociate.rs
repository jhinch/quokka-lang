//! Folds the constants in chains of binary instructions, e.g. `((n + 3) + m) + 5` becomes
//! `(n + m) + 8`.
//!
//! For `t1 = a OP1 c1` and `t2 = t1 OP2 x`, where `c1` is a constant and `OP1` and `OP2`
//! right-commute (see `OpKind::right_commutes_with`):
//! - **Fold:** if `x` is a constant, `t2` is rewritten to `a OP1 c` with `c` computed at compile
//!   time, e.g. `(n - 3) - 5` becomes `n - 8`. `t1` is unchanged, and becomes dead if `t2` was
//!   its only use.
//! - **Swap:** otherwise the pair is rewritten to `t1 = a OP2 x` and `t2 = t1 OP1 c1`, moving the
//!   constant outward where a later fold can reach it. Each opcode keeps its own right operand,
//!   so `(n - 3) + m` becomes `(n + m) - 3`. This requires that `t1` has exactly one use (`t2`),
//!   since its value changes, that `x`'s definition dominates `t1`'s, since that is where
//!   `x` is now read, and that `t1` is not live at any `Breakpoint`, since a breakpoint would
//!   observe the changed value.
//!
//! If `OP2` is commutative, `t1` may instead be its right operand (`t2 = x OP2 t1`).

use super::{Context, Rewrite, Rule, to_literal};
use crate::consteval::Value;
use crate::mir::{BinOp, BlockId, Function, Instr, OpKind, Operand};
use indexmap::IndexSet;

pub(super) struct Reassociate;

impl Rule for Reassociate {
    fn name(&self) -> &'static str {
        "reassociate"
    }

    /// Tries to find a rewrite for the outer instruction at `outer_position`: a local operand of
    /// that instruction whose defining instruction is a right-commuting binary op holding a
    /// constant. If the outer instruction's other operand is also a constant the two are folded,
    /// otherwise the other operand is swapped into the defining instruction's position.
    fn rewrites(&self, ctx: &mut Context, outer_position: (BlockId, usize)) -> Vec<Rewrite> {
        let function = &*ctx.function;
        let Some(outer) =
            function.blocks[outer_position.0].instructions[outer_position.1].as_binop()
        else {
            return vec![];
        };
        let ty = ctx.operand_ty(&outer);

        // `left` is always eligible as the chain link; `right` only when `op` is commutative.
        let mut candidates = vec![(outer.left, outer.right.clone())];
        if outer.op.is_commutative()
            && let Operand::Local(right) = outer.right
        {
            candidates.push((right, Operand::Local(outer.left)));
        }

        for (link, other) in candidates {
            // SSA guarantees `link`'s definition dominates its use by the outer instruction,
            // wherever in the function that definition is.
            let Some(inner_position) = function.defined_at(link) else {
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
            let Some(c1) = ctx.constant(&inner.right, ty) else {
                continue;
            };

            if let Some(c2) = ctx.constant(&other, ty) {
                // `(a OP1 c1) OP2 c2` => `a OP1 (c1 ⊕ c2)`. Only the outer instruction changes,
                // so other uses of `t1` are unaffected.
                let folded = fold_constants(inner.op, outer.op, c1, c2);
                return vec![Rewrite::replace(
                    outer_position,
                    BinOp {
                        op: inner.op,
                        target: outer.target,
                        left: inner.left,
                        right: to_literal(folded),
                    }
                    .into_instr(),
                )];
            }

            // `(a OP1 c) OP2 x` => `(a OP2 x) OP1 c`. `x` moves to the inner instruction's
            // position, so its definition must dominate that position too, not just the outer
            // instruction's.
            let Operand::Local(other) = other else {
                continue;
            };
            match function.defined_at(other) {
                Some(other_defined) if ctx.dominators.dominates(other_defined, inner_position) => {}
                _ => continue,
            }
            // `t1`'s value changes, so the outer instruction must be its only use.
            ctx.debug_assert_use_count(inner.target);
            if ctx.use_count(inner.target) != 1 {
                continue;
            }
            // A breakpoint observes every live value, so `t1` must not be live at one.
            if live_at_breakpoint(function, inner_position, outer_position) {
                continue;
            }
            return vec![
                Rewrite::replace(
                    inner_position,
                    BinOp {
                        op: outer.op,
                        target: inner.target,
                        left: inner.left,
                        right: Operand::Local(other),
                    }
                    .into_instr(),
                ),
                Rewrite::replace(
                    outer_position,
                    BinOp {
                        op: inner.op,
                        target: outer.target,
                        left: inner.target,
                        right: inner.right,
                    }
                    .into_instr(),
                ),
            ];
        }
        vec![]
    }
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
