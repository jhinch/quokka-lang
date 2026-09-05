use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::mir::{Function, Global, GlobalId, Mir};
use crate::multi::MultiError;
use crate::ty::TyRegistry;
use slotmap::SlotMap;
use std::ops::{BitOr, BitOrAssign};
use tracing::instrument;

pub mod adce;
pub mod graph;
pub mod prune;
pub mod sccp;

pub struct Optimizer {
    features: PreviewFeatureSet,
    max_rounds: usize,
}

impl Default for Optimizer {
    fn default() -> Self {
        Optimizer {
            features: PreviewFeatureSet::default(),
            max_rounds: 10,
        }
    }
}

impl Optimizer {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    fn initialize_optimizations(&self) -> Vec<Box<dyn Optimize>> {
        let mut optimizations = Vec::<Box<dyn Optimize>>::new();
        if self.features.is_enabled(PreviewFeature::Adce) {
            optimizations.push(Box::new(adce::Adce));
        }
        if self.features.is_enabled(PreviewFeature::Sccp) {
            optimizations.push(Box::new(sccp::Sccp));
        }
        if self.features.is_enabled(PreviewFeature::Prune) {
            optimizations.push(Box::new(prune::Prune));
        }
        optimizations
    }

    #[instrument(skip_all)]
    pub fn optimize(&self, ty_registry: &TyRegistry, mir: &mut Mir) -> Result<(), MultiError> {
        let optimizations = self.initialize_optimizations();
        if optimizations.is_empty() {
            return Ok(());
        }
        for i in 0..self.max_rounds {
            if self.optimization_round(&optimizations, ty_registry, i, mir) == Outcome::Unmodified {
                break;
            }
            for function in mir.functions.values() {
                function.check_integrity()?;
            }
        }
        Ok(())
    }

    #[instrument(skip_all, fields(i = %i))]
    fn optimization_round(
        &self,
        optimizations: &[Box<dyn Optimize>],
        ty_registry: &TyRegistry,
        i: usize,
        mir: &mut Mir,
    ) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for optimization in optimizations {
            outcome |= optimization.optimize(mir, ty_registry);
        }
        outcome
    }
}

pub trait Optimize {
    #[instrument(skip_all)]
    fn optimize(&self, mir: &mut Mir, ty_registry: &TyRegistry) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for function in mir.functions.values_mut() {
            outcome |= self.optimize_function(function, &mir.globals, ty_registry);
        }
        outcome
    }

    fn optimize_function(
        &self,
        _function: &mut Function,
        _globals: &SlotMap<GlobalId, Global>,
        _ty_registry: &TyRegistry,
    ) -> Outcome {
        Outcome::Unmodified
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Unmodified,
    Modified,
}

impl BitOr for Outcome {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Outcome::Unmodified, Outcome::Unmodified) => Outcome::Unmodified,
            _ => Outcome::Modified,
        }
    }
}

impl BitOrAssign for Outcome {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Identifier, IntegerLiteral};
    use crate::mir::{FunctionBuilder, Instr, MirBuilder, Operand, Terminator};
    use crate::ty::Ty;

    fn new_builder(name: &str) -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from(name), Ty::U8)
    }

    /// `let _dead = 1 + 1; return 0;` where `_dead`'s result is never used, plus a literal
    /// `Nop`. Adce alone turns the dead `Add` into a `Nop` but does not shrink the block; Prune
    /// alone removes `Nop`s but cannot know the `Add` is dead; only both together (in the same
    /// round, since `Adce` runs before `Prune`) fully remove it.
    fn function_with_dead_add_and_nop() -> Function {
        let mut fb = new_builder("f");
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let one = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: one,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        let dead = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Add {
                target: dead,
                left: one,
                right: Operand::Local(one),
            },
        );
        fb.push_instr(block, Instr::Nop);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        fb.build()
    }

    fn single_function_mir(function: Function) -> (Mir, TyRegistry) {
        let mut builder = MirBuilder::default();
        let id = builder.reserve_function();
        let mut fb = FunctionBuilder::new(function.name.clone(), function.return_ty);
        // rebuild via the real builder API isn't needed here: Optimizer::optimize only reads
        // mir.functions, so we can seal a trivially-built function and then overwrite it.
        let block = fb.new_block();
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        builder.seal_function(id, fb);
        let mut mir = builder.build();
        mir.functions[id] = function;
        (mir, TyRegistry::default())
    }

    fn instr_count(mir: &Mir) -> usize {
        mir.functions
            .values()
            .flat_map(|f| f.blocks.values())
            .map(|b| b.instructions.len())
            .sum()
    }

    fn nop_count(mir: &Mir) -> usize {
        mir.functions
            .values()
            .flat_map(|f| f.blocks.values())
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instr::Nop))
            .count()
    }

    // --- Outcome ---

    #[test]
    fn outcome_bitor_is_modified_unless_both_unmodified() {
        assert_eq!(
            Outcome::Unmodified | Outcome::Unmodified,
            Outcome::Unmodified
        );
        assert_eq!(Outcome::Unmodified | Outcome::Modified, Outcome::Modified);
        assert_eq!(Outcome::Modified | Outcome::Unmodified, Outcome::Modified);
        assert_eq!(Outcome::Modified | Outcome::Modified, Outcome::Modified);
    }

    #[test]
    fn outcome_bitor_assign_matches_bitor() {
        let mut outcome = Outcome::Unmodified;
        outcome |= Outcome::Modified;
        assert_eq!(outcome, Outcome::Modified);
        outcome |= Outcome::Unmodified;
        assert_eq!(outcome, Outcome::Modified);
    }

    // --- Optimize trait default `optimize` (fans out to every function in the Mir) ---

    struct MarkFirstInstrNop;

    impl Optimize for MarkFirstInstrNop {
        fn optimize_function(
            &self,
            function: &mut Function,
            _globals: &SlotMap<GlobalId, Global>,
            _ty_registry: &TyRegistry,
        ) -> Outcome {
            for block in function.blocks.values_mut() {
                if let Some(first) = block.instructions.first_mut()
                    && !matches!(first, Instr::Nop)
                {
                    *first = Instr::Nop;
                    return Outcome::Modified;
                }
            }
            Outcome::Unmodified
        }
    }

    #[test]
    fn optimize_trait_default_visits_every_function_in_the_mir() {
        let (mut mir, ty_registry) = single_function_mir(function_with_dead_add_and_nop());
        let outcome = MarkFirstInstrNop.optimize(&mut mir, &ty_registry);
        assert_eq!(outcome, Outcome::Modified);
        assert_eq!(
            nop_count(&mir),
            2,
            "first instr replaced, plus the original literal Nop"
        );
    }

    // --- Optimizer::optimize (feature gating + round-based fixpoint) ---

    #[test]
    fn optimizer_with_no_features_enabled_is_a_noop() {
        let (mut mir, ty_registry) = single_function_mir(function_with_dead_add_and_nop());
        let before = instr_count(&mir);
        Optimizer::default()
            .optimize(&ty_registry, &mut mir)
            .unwrap();
        assert_eq!(instr_count(&mir), before);
        assert_eq!(
            nop_count(&mir),
            1,
            "the literal Nop is untouched without Prune enabled"
        );
    }

    #[test]
    fn optimizer_only_runs_explicitly_enabled_passes() {
        let (mut mir, ty_registry) = single_function_mir(function_with_dead_add_and_nop());
        let mut optimizer = Optimizer::default();
        optimizer.enable_feature(PreviewFeature::Prune);
        optimizer.optimize(&ty_registry, &mut mir).unwrap();
        // Prune removes the literal Nop but cannot know the Add is dead (that's Adce's job)
        assert_eq!(nop_count(&mir), 0);
        let has_dead_add = mir
            .functions
            .values()
            .flat_map(|f| &f.blocks)
            .flat_map(|(_, b)| &b.instructions)
            .any(|i| matches!(i, Instr::Add { .. }));
        assert!(
            has_dead_add,
            "Prune alone must not remove non-Nop dead code"
        );
    }

    #[test]
    fn optimizer_combines_adce_and_prune_across_rounds_to_fully_clean_up() {
        let (mut mir, ty_registry) = single_function_mir(function_with_dead_add_and_nop());
        let mut optimizer = Optimizer::default();
        optimizer.enable_feature(PreviewFeature::Adce);
        optimizer.enable_feature(PreviewFeature::Prune);
        optimizer.optimize(&ty_registry, &mut mir).unwrap();
        assert_eq!(nop_count(&mir), 0);
        let has_dead_add = mir
            .functions
            .values()
            .flat_map(|f| &f.blocks)
            .flat_map(|(_, b)| &b.instructions)
            .any(|i| matches!(i, Instr::Add { .. }));
        assert!(
            !has_dead_add,
            "Adce marks the dead Add as Nop and Prune sweeps it away"
        );
    }

    #[test]
    fn enable_features_extends_existing_set() {
        let mut optimizer = Optimizer::default();
        optimizer.enable_feature(PreviewFeature::Adce);
        optimizer.enable_features([PreviewFeature::Prune].into_iter().collect());
        let optimizations = optimizer.initialize_optimizations();
        assert_eq!(optimizations.len(), 2);
    }
}
