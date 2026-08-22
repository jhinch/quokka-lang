use crate::mir::Mir;
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use tracing::instrument;

/// Aggressive Dead Code Elimination (ADCE)
pub struct Adce;

impl Optimize for Adce {
    #[instrument(skip(self, _mir, _ty_registry))]
    fn optimize(&self, _mir: &mut Mir, _ty_registry: &TyRegistry) -> Outcome {
        Outcome::Unmodified
    }
}
