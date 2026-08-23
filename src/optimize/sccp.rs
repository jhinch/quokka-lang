use crate::mir::Function;
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use tracing::instrument;

/// Sparse Conditional Constant Propagation
pub struct Sccp;

impl Optimize for Sccp {
    #[instrument(skip(self, _function, _ty_registry), fields(function = %_function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(&self, _function: &mut Function, _ty_registry: &TyRegistry) -> Outcome {
        // todo
        Outcome::Unmodified
    }
}
