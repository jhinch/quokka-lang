use crate::mir::Mir;
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use tracing::instrument;

/// Sparse Conditional Constant Propagation
pub struct Sccp;

impl Optimize for Sccp {
    #[instrument(skip(self, _mir, _ty_registry))]
    fn optimize(&self, _mir: &mut Mir, _ty_registry: &TyRegistry) -> Outcome {
        // todo
        Outcome::Unmodified
    }
}
