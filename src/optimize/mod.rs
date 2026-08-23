use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::mir::{Function, Mir};
use crate::multi::MultiError;
use crate::ty::TyRegistry;
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

    #[instrument(skip(self, ty_registry, mir))]
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

    #[instrument(skip(self, optimizations, ty_registry, mir))]
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
    #[instrument(skip(self, mir, ty_registry))]
    fn optimize(&self, mir: &mut Mir, ty_registry: &TyRegistry) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for function in mir.functions.values_mut() {
            outcome |= self.optimize_function(function, ty_registry);
        }
        outcome
    }

    fn optimize_function(&self, _function: &mut Function, _ty_registry: &TyRegistry) -> Outcome {
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
