use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{
    BinOp, BlockId, Function, FunctionMutator, Global, GlobalId, Instr, LocalId, Operand, Splice,
};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::{Ty, TyRegistry};
use indexmap::{IndexMap, IndexSet};
use slotmap::{SecondaryMap, SlotMap};
use std::ops::Range;
use tracing::{instrument, trace};

mod additive;
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
/// A rewrite that replaces one instruction with another defining the same local is applied
/// immediately. Any other rewrite inserts, removes, or redefines instructions, which would shift
/// the positions the rest of the sweep relies on, so it is held back and applied when the sweep
/// ends. A constant is a literal operand, or a local defined by a `Copy` of a literal.
pub struct Canonicalize;

/// In application order at each instruction position.
const RULES: &[&dyn Rule] = &[
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
        let mut ctx = Context {
            // Rewrites never change the CFG, so the block-level dominators stay valid for the
            // whole pass. Within a block, dominance is read off instruction positions, which
            // only move between sweeps.
            dominators: Dominators::new(function),
            function,
            ty_registry,
            use_counts: SecondaryMap::new(),
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
            // The use counts are only refreshed between sweeps. Rules must tolerate them being
            // stale within a sweep, which a later sweep then makes up for.
            ctx.use_counts = ctx.function.count_uses();
            let mut swept = Outcome::Unmodified;
            let mut deferred = Deferred::default();
            for block_id in ctx.function.blocks.keys().collect::<Vec<_>>() {
                for index in 0..ctx.function.blocks[block_id].instructions.len() {
                    let position = (block_id, index);
                    for rule in RULES {
                        // The instruction here is already being replaced when the sweep ends.
                        if deferred.covers(position) {
                            break;
                        }
                        let rewrites = rule.rewrites(&mut ctx, position);
                        // A rule's rewrites only make sense together.
                        if rewrites.iter().any(|rewrite| deferred.conflicts(rewrite)) {
                            continue;
                        }
                        for rewrite in rewrites {
                            trace!(rule = rule.name(), ?rewrite, "rewrite");
                            if let Err(rewrite) = rewrite.apply_in_place(ctx.function) {
                                deferred.push(rewrite);
                            }
                            swept = Outcome::Modified;
                        }
                    }
                }
            }
            ctx.dangling |= !deferred.splices.is_empty();
            for (block, splices) in deferred.splices {
                FunctionMutator::from(&mut *ctx.function).splice_instrs(block, splices);
            }
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
    /// Positions do not move during a sweep, so a rewrite that is not a one-for-one replacement
    /// defining the same local only takes effect once the sweep ends (see [`Canonicalize`]).
    /// Until then the rest of the sweep still reads the instructions it replaces, so every local
    /// they define must either keep its value or have no remaining use. A local made with
    /// [`Context::fresh_local`] has no definition to look up before the next sweep.
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

    /// Applies the rewrite if no instruction moves and no definition changes, which is when one
    /// instruction is replaced by another defining the same local. Hands the rewrite back
    /// otherwise.
    fn apply_in_place(mut self, function: &mut Function) -> Result<(), Self> {
        let instructions = &mut function.blocks[self.block].instructions;
        if let ([old], [new]) = (
            &mut instructions[self.splice.range.clone()],
            self.splice.instrs.as_mut_slice(),
        ) && old.target_local() == new.target_local()
        {
            std::mem::swap(old, new);
            return Ok(());
        }
        Err(self)
    }
}

/// The rewrites held back until the end of a sweep.
#[derive(Default)]
struct Deferred {
    splices: IndexMap<BlockId, Vec<Splice>>,
    /// Every position inside the range of a held back splice.
    covered: IndexSet<(BlockId, usize)>,
}

impl Deferred {
    fn covers(&self, position: (BlockId, usize)) -> bool {
        self.covered.contains(&position)
    }

    /// Whether `rewrite` touches an instruction a held back splice already replaces. An
    /// insertion conflicts if it falls between two such instructions, which may be inside one
    /// splice's range.
    fn conflicts(&self, rewrite: &Rewrite) -> bool {
        let Range { start, end } = rewrite.splice.range;
        if start == end {
            return start > 0
                && self.covers((rewrite.block, start - 1))
                && self.covers((rewrite.block, start));
        }
        (start..end).any(|index| self.covers((rewrite.block, index)))
    }

    fn push(&mut self, rewrite: Rewrite) {
        let Rewrite { block, splice } = rewrite;
        self.covered
            .extend(splice.range.clone().map(|index| (block, index)));
        self.splices.entry(block).or_default().push(splice);
    }
}
