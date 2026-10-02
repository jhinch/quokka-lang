use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{
    BinOp, BlockId, Function, FunctionMutator, Global, GlobalId, Instr, LocalId, Operand, Splice,
};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use slotmap::{SecondaryMap, SlotMap};
use std::ops::Range;
use tracing::{instrument, trace};

mod additive;
mod commute;
mod compare;
mod identity;
mod neg;
mod not;
mod reassociate;
mod signed_compare;

/// Canonicalize
///
/// Rewrites instructions toward one normal form per operation. Each rule lives in its own
/// submodule and proposes replacements for one instruction position at a time. The rules are run
/// over the whole function repeatedly until none of them fires, so one rule's output can feed
/// another within a single invocation of the pass.
///
/// A sweep visits the reachable blocks in reverse post-order, so a definition is always visited
/// before any instruction that reads it other than a `Phi`. Every rewrite is applied as soon as
/// its rule proposes it, with the use counts and the definition positions kept up to date, so the
/// rules that follow always see the function as it is. A constant is a literal operand, or a
/// local defined by a `Copy` of a literal.
pub struct Canonicalize;

/// In application order at each instruction position.
const RULES: &[&dyn Rule] = &[
    &commute::Commute,
    &additive::Additive,
    &not::Not,
    &neg::Neg,
    &reassociate::Reassociate,
    &compare::Compare,
    &signed_compare::SignedCompare,
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
        // Rewrites never change the CFG, so the block order and the block-level dominators stay
        // valid for the whole pass. Within a block, dominance is read off instruction positions.
        let graph = Graph::from(&*function);
        let mut order = graph
            .postorder()
            .filter_map(|node| match node {
                Node::Block(block) => Some(block),
                Node::Exit => None,
            })
            .collect::<Vec<_>>();
        order.reverse();
        let mut ctx = Context {
            dominators: Dominators(graph.calculate_immediate_dominators()),
            // Counted once here and then kept current by every rewrite applied.
            use_counts: function.count_uses(),
            function,
            ty_registry,
            dangling: false,
        };
        let mut outcome = Outcome::Unmodified;
        for sweep in 0.. {
            if sweep == MAX_SWEEPS {
                debug_assert!(
                    false,
                    "canonicalize did not converge in {MAX_SWEEPS} sweeps"
                );
                break;
            }
            let mut swept = Outcome::Unmodified;
            for &block_id in &order {
                let mut index = 0;
                while index < ctx.function.blocks[block_id].instructions.len() {
                    for rule in RULES {
                        let rewrites = rule.rewrites(&mut ctx, (block_id, index));
                        if rewrites.is_empty() {
                            continue;
                        }
                        trace!(rule = rule.name(), ?rewrites, "rewrite");
                        // The remaining rules look at whatever now stands where the instruction
                        // was, and the instructions after it are visited in turn.
                        index = ctx.apply(rewrites, (block_id, index));
                        swept = Outcome::Modified;
                    }
                    index += 1;
                }
            }
            ctx.debug_assert_use_counts();
            if swept == Outcome::Unmodified {
                break;
            }
            outcome = Outcome::Modified;
        }
        if ctx.dangling {
            ctx.function.remove_dangling();
        }
        outcome
    }
}

/// A canonicalization rule.
trait Rule: Sync {
    /// Identifies the rule in traces.
    fn name(&self) -> &'static str;

    /// The rewrites, if any, that put the instruction at `position` into this rule's normal form.
    /// They may also touch other instructions.
    ///
    /// The rewrites are applied together, before any other rule runs, so their ranges all refer
    /// to the function as it is during this call, and those in the same block must not overlap.
    /// [`Context::use_count`] is exact, so a rewrite changing the value of a local must check
    /// that the instruction it rewrites alongside is that local's only reader.
    fn rewrites(&self, ctx: &mut Context, position: (BlockId, usize)) -> Vec<Rewrite>;
}

/// The function a [`Rule`] is looking at, with the def-use machinery the rules share.
struct Context<'a> {
    function: &'a mut Function,
    ty_registry: &'a TyRegistry,
    dominators: Dominators,
    use_counts: SecondaryMap<LocalId, u32>,
    /// Whether the function may hold locals without a definition, to clean up when the pass ends.
    dangling: bool,
}

impl Context<'_> {
    /// A new local for a rewrite to define. A rule should only make one for a rewrite it goes on
    /// to return.
    fn fresh_local(&mut self, ty: Ty) -> LocalId {
        self.dangling = true;
        FunctionMutator::from(&mut *self.function).new_local(ty)
    }

    /// How many times `local` is read by the function as it is now.
    fn use_count(&self, local: LocalId) -> u32 {
        self.use_counts.get(local).copied().unwrap_or(0)
    }

    /// Applies the `rewrites` of one rule, keeping the use counts current. Returns where the
    /// instruction at `position` is afterward: the start of what replaced it, if it was replaced.
    fn apply(&mut self, rewrites: Vec<Rewrite>, (at_block, at_index): (BlockId, usize)) -> usize {
        let mut index = at_index;
        let mut blocks = IndexMap::<BlockId, Vec<Splice>>::new();
        for Rewrite { block, splice } in rewrites {
            let removed = &self.function.blocks[block].instructions[splice.range.clone()];
            for source in removed.iter().flat_map(Instr::source_locals) {
                self.use_counts[source] -= 1;
            }
            for source in splice.instrs.iter().flat_map(Instr::source_locals) {
                *self.use_counts.entry(source).unwrap().or_insert(0) += 1;
            }
            // A definition removed without a replacement leaves its local dangling.
            self.dangling |= removed
                .iter()
                .filter_map(Instr::target_local)
                .any(|target| {
                    !splice
                        .instrs
                        .iter()
                        .any(|instr| instr.target_local() == Some(target))
                });
            if block == at_block {
                let Range { start, end } = splice.range;
                if end <= at_index {
                    index = index + splice.instrs.len() - (end - start);
                } else if start <= at_index {
                    index -= at_index - start;
                }
            }
            blocks.entry(block).or_default().push(splice);
        }
        for (block, splices) in blocks {
            FunctionMutator::from(&mut *self.function).splice_instrs(block, splices);
        }
        index
    }

    /// Checks the incrementally maintained use count of every local against a recount.
    fn debug_assert_use_counts(&self) {
        if cfg!(debug_assertions) {
            let recount = self.function.count_uses();
            for local in self.function.locals.keys() {
                assert_eq!(
                    self.use_count(local),
                    recount.get(local).copied().unwrap_or(0),
                    "use count of {local:?} in {} drifted from a recount",
                    self.function.name
                );
            }
        }
    }

    /// Checks the incrementally maintained use count of `local` against a recount.
    fn debug_assert_use_count(&self, local: LocalId) {
        debug_assert_eq!(
            self.use_count(local),
            self.function.count_uses().get(local).copied().unwrap_or(0),
            "use count of {local:?} in {} drifted from a recount",
            self.function.name
        );
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

/// A [`Splice`] of the instructions of `block`.
#[derive(Debug)]
struct Rewrite {
    block: BlockId,
    splice: Splice,
}

impl Rewrite {
    /// Replaces the instruction at `position` with `instr`.
    fn replace(position: (BlockId, usize), instr: Instr) -> Self {
        Self::replace_with(position, vec![instr])
    }

    /// Replaces the instruction at `position` with `instrs`, in order.
    fn replace_with((block, index): (BlockId, usize), instrs: Vec<Instr>) -> Self {
        Rewrite {
            block,
            splice: Splice {
                range: index..index + 1,
                instrs,
            },
        }
    }
}
