use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{BinOp, BlockId, Function, Global, GlobalId, Instr, LocalId, Operand};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use slotmap::{SecondaryMap, SlotMap};
use tracing::{instrument, trace};

mod additive;
mod compare;
mod identity;
mod neg;
mod not;
mod reassociate;

/// Canonicalize
///
/// Rewrites instructions toward one normal form per operation. Each rule lives in its own
/// submodule and proposes replacements for one instruction position at a time. The rules are run
/// over the whole function repeatedly until none of them fires, so one rule's output can feed
/// another within a single invocation of the pass.
///
/// Instructions are never inserted, removed, or moved; only operands and opcodes change. A
/// constant is a literal operand, or a local defined by a `Copy` of a literal.
pub struct Canonicalize;

/// In application order at each instruction position.
const RULES: &[&dyn Rule] = &[
    &additive::Additive,
    &not::Not,
    &neg::Neg,
    &reassociate::Reassociate,
    &compare::Compare,
    &identity::Identity,
];

/// A bound on the sweeps over a function. Rules are expected to converge long before this; hitting
/// it means two rewrites undo each other.
const MAX_SWEEPS: usize = 64;

impl Optimize for Canonicalize {
    #[instrument(name = "canonicalize", skip_all, fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(
        &self,
        function: &mut Function,
        _globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
    ) -> Outcome {
        // Rewrites never change the CFG or where any local is defined, so the dominators stay
        // valid for the whole pass.
        let dominators = Dominators::new(function);
        let mut outcome = Outcome::Unmodified;
        for sweep in 0.. {
            if sweep == MAX_SWEEPS {
                debug_assert!(
                    false,
                    "canonicalize did not converge in {MAX_SWEEPS} sweeps"
                );
                break;
            }
            // The use counts are only refreshed between sweeps. Rules must tolerate them being
            // stale within a sweep, which a later sweep then makes up for.
            let use_counts = function.count_uses();
            let mut swept = Outcome::Unmodified;
            for block_id in function.blocks.keys().collect::<Vec<_>>() {
                for index in 0..function.blocks[block_id].instructions.len() {
                    let position = (block_id, index);
                    for rule in RULES {
                        let ctx = Context {
                            function,
                            ty_registry,
                            dominators: &dominators,
                            use_counts: &use_counts,
                        };
                        for rewrite in rule.rewrites(&ctx, position) {
                            trace!(rule = rule.name(), ?rewrite, "rewrite");
                            rewrite.apply(function);
                            swept = Outcome::Modified;
                        }
                    }
                }
            }
            if swept == Outcome::Unmodified {
                break;
            }
            outcome = Outcome::Modified;
        }
        outcome
    }
}

/// A canonicalization rule.
trait Rule: Sync {
    /// Identifies the rule in traces.
    fn name(&self) -> &'static str;

    /// The rewrites, if any, that put the instruction at `position` into this rule's normal form.
    /// They may also replace other instructions, but must leave every instruction defining the
    /// same local it did before.
    fn rewrites(&self, ctx: &Context, position: (BlockId, usize)) -> Vec<Rewrite>;
}

/// The function a [`Rule`] is looking at, with the def-use machinery the rules share.
struct Context<'a> {
    function: &'a Function,
    ty_registry: &'a TyRegistry,
    dominators: &'a Dominators,
    use_counts: &'a SecondaryMap<LocalId, u32>,
}

impl Context<'_> {
    /// How many times `local` is read. See [`Canonicalize::optimize_function`] for staleness.
    fn use_count(&self, local: LocalId) -> u32 {
        self.use_counts.get(local).copied().unwrap_or(0)
    }

    /// The type `binop`'s operands are read at. This is the type to decode its constants with,
    /// rather than its target's type: the two differ for a comparison, whose target is a `u1`.
    fn operand_ty(&self, binop: &BinOp) -> Ty {
        self.function.locals[binop.left].ty
    }

    /// The instruction defining `operand`, if it is a local with a definition.
    fn definition(&self, operand: &Operand) -> Option<&Instr> {
        let Operand::Local(local) = operand else {
            return None;
        };
        let (block, index) = self.function.defined_at(*local)?;
        Some(&self.function.blocks[block].instructions[index])
    }

    /// The compile-time constant value of `operand` read as a `ty`, if it has one: either a
    /// literal embedded directly in the instruction, or a local whose sole definition is a `Copy`
    /// of a literal. The analyzer always lowers a source-level literal used as a binary operand
    /// into the latter shape (`#n = copy 5; x = add y, #n`) rather than embedding it directly, so
    /// matching only `Operand::Literal` would never fire on freshly analyzed MIR.
    fn constant(&self, operand: &Operand, ty: Ty) -> Option<Value> {
        let literal = match operand {
            Operand::Literal(literal) => literal,
            Operand::Local(local) => {
                let (block, index) = self.function.defined_at(*local)?;
                match &self.function.blocks[block].instructions[index] {
                    Instr::Copy {
                        source: Operand::Literal(literal),
                        ..
                    } => literal,
                    _ => return None,
                }
            }
            Operand::FnPtr(_) => return None,
        };
        let bytes = ty.bit_size(self.ty_registry).padded_bytes() as usize;
        Some(Value::from_bytes(
            ty,
            self.ty_registry,
            &literal.value.to_le_bytes()[..bytes],
        ))
    }
}

fn to_literal(value: Value) -> Operand {
    let mut bytes = value.into_vec();
    bytes.resize(4, 0);
    Operand::Literal(IntegerLiteral::from(u32::from_le_bytes(
        *bytes.as_array::<4>().unwrap(),
    )))
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

/// A replacement instruction, at the `(block, index)` of the instruction it replaces.
#[derive(Debug)]
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
