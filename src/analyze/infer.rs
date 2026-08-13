use crate::ty::Ty;
use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredTy {
    Variable(TyVar),
    Concrete(Ty),
}

impl InferredTy {
    #[must_use]
    pub const fn to_concrete(&self) -> Option<Ty> {
        match self {
            InferredTy::Concrete(ty) => Some(*ty),
            InferredTy::Variable(_) => None,
        }
    }

    #[must_use]
    pub const fn to_variable(&self) -> Option<TyVar> {
        match self {
            InferredTy::Variable(ty_var) => Some(*ty_var),
            InferredTy::Concrete(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVar(u32);

#[derive(Default)]
pub struct Inferrer {
    substitutions: IndexMap<TyVar, InferredTy>,
    next_ty_var: u32,
}

#[derive(Debug)]
pub struct UnificationError {
    pub ty1: InferredTy,
    pub ty2: InferredTy,
}

impl Inferrer {
    pub fn fresh(&mut self) -> TyVar {
        let var = TyVar(self.next_ty_var);
        self.next_ty_var += 1;
        var
    }

    pub fn unify(
        &mut self,
        mut ty1: InferredTy,
        mut ty2: InferredTy,
    ) -> Result<(), UnificationError> {
        self.prune(&mut ty1);
        self.prune(&mut ty2);
        match (ty1, ty2) {
            (InferredTy::Variable(ty1), InferredTy::Variable(ty2)) if ty1 == ty2 => Ok(()),
            (InferredTy::Variable(var), other) | (other, InferredTy::Variable(var)) => {
                self.bind_without_pruning(var, other);
                Ok(())
            }
            (InferredTy::Concrete(Ty::ERROR), _) | (_, InferredTy::Concrete(Ty::ERROR)) => {
                // This branch intentionally ignores Ty::NEVER, allowing it to be checked by a simple equality check.
                // It isn't a true bottom type, constraining how never types coerce to other types.
                // For example:
                //      let a: u8 = 1;
                //      let b = a + (loop {});
                // should intentionally produce a type mismatch error. Currently only branching
                // expressions like if-else coerce
                Ok(())
            }
            (InferredTy::Concrete(ty1), InferredTy::Concrete(ty2)) if ty1 == ty2 => Ok(()),
            (ty1, ty2) => Err(UnificationError { ty1, ty2 }),
        }
    }

    pub fn prune(&mut self, ty: &mut InferredTy) {
        let mut original_var = if let InferredTy::Variable(var) = ty {
            *var
        } else {
            return;
        };
        match self.substitutions.get(&original_var) {
            None => (),
            Some(InferredTy::Variable(sub_ty_var)) => {
                let mut sub_ty_var = *sub_ty_var;
                let mut target_ty_var = sub_ty_var;
                while let Some(InferredTy::Variable(ty_var)) =
                    self.substitutions.get(&target_ty_var)
                {
                    target_ty_var = *ty_var;
                }
                while sub_ty_var != target_ty_var {
                    // flatten:
                    // v1 -> v2 -> v3       to v1 -> v3
                    // or
                    // v1 -> v2 -> v3 -> T  to v1 -> v3 -> T
                    if let Some(InferredTy::Variable(ty_var)) =
                        self.substitutions.get(&original_var)
                    {
                        let ty_var = *ty_var;
                        self.substitutions[&original_var] = InferredTy::Variable(target_ty_var);
                        original_var = sub_ty_var;
                        sub_ty_var = ty_var;
                    } else {
                        break;
                    }
                }
                *ty = match self.substitutions.get(&target_ty_var) {
                    None => InferredTy::Variable(target_ty_var),
                    Some(sub_ty) => sub_ty.clone(),
                };
            }
            Some(sub_ty) => {
                *ty = sub_ty.clone();
            }
        }
    }

    #[must_use]
    pub fn resolve(&self, mut var: TyVar) -> Option<Ty> {
        loop {
            match self.substitutions.get(&var)? {
                InferredTy::Variable(v) => var = *v,
                InferredTy::Concrete(ty) => return Some(*ty),
            }
        }
    }

    pub fn bind(&mut self, var: TyVar, mut other: InferredTy) {
        self.prune(&mut other);
        self.bind_without_pruning(var, other);
    }

    fn bind_without_pruning(&mut self, var: TyVar, other: InferredTy) {
        assert!(
            !self.occurs(var, &other),
            "{var:?} occurs in {other:?} however quokka does not syntactically support cyclic type definitions"
        );
        self.substitutions.insert(var, other);
    }

    fn occurs(&self, var: TyVar, other: &InferredTy) -> bool {
        match other {
            InferredTy::Variable(v) => *v == var,
            InferredTy::Concrete(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_returns_distinct_ty_vars() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        assert_ne!(a, b);
    }

    #[test]
    fn to_concrete_and_to_variable() {
        let concrete = InferredTy::Concrete(Ty::U8);
        assert_eq!(concrete.to_concrete(), Some(Ty::U8));
        assert_eq!(concrete.to_variable(), None);

        let mut inferrer = Inferrer::default();
        let var = InferredTy::Variable(inferrer.fresh());
        assert_eq!(var.to_concrete(), None);
        assert!(var.to_variable().is_some());
    }

    #[test]
    fn unify_equal_concrete_tys_succeeds() {
        let mut inferrer = Inferrer::default();
        let result = inferrer.unify(InferredTy::Concrete(Ty::U8), InferredTy::Concrete(Ty::U8));
        assert!(result.is_ok());
    }

    #[test]
    fn unify_different_concrete_tys_fails() {
        let mut inferrer = Inferrer::default();
        let result = inferrer.unify(InferredTy::Concrete(Ty::U8), InferredTy::Concrete(Ty::U16));
        let err = result.unwrap_err();
        assert_eq!(err.ty1, InferredTy::Concrete(Ty::U8));
        assert_eq!(err.ty2, InferredTy::Concrete(Ty::U16));
    }

    #[test]
    fn unify_error_ty_is_a_wildcard() {
        let mut inferrer = Inferrer::default();
        assert!(
            inferrer
                .unify(
                    InferredTy::Concrete(Ty::ERROR),
                    InferredTy::Concrete(Ty::U16)
                )
                .is_ok()
        );
        assert!(
            inferrer
                .unify(
                    InferredTy::Concrete(Ty::U16),
                    InferredTy::Concrete(Ty::ERROR)
                )
                .is_ok()
        );
    }

    #[test]
    fn unify_never_ty_is_not_a_wildcard() {
        // NEVER is intentionally not special-cased like ERROR: it must match exactly.
        let mut inferrer = Inferrer::default();
        let result = inferrer.unify(
            InferredTy::Concrete(Ty::NEVER),
            InferredTy::Concrete(Ty::U16),
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_same_variable_with_itself_succeeds_without_binding() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        let result = inferrer.unify(InferredTy::Variable(var), InferredTy::Variable(var));
        assert!(result.is_ok());
        assert_eq!(inferrer.resolve(var), None);
    }

    #[test]
    fn unify_variable_with_concrete_binds_variable() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        inferrer
            .unify(InferredTy::Variable(var), InferredTy::Concrete(Ty::U8))
            .unwrap();
        assert_eq!(inferrer.resolve(var), Some(Ty::U8));
    }

    #[test]
    fn unify_concrete_with_variable_binds_variable() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        inferrer
            .unify(InferredTy::Concrete(Ty::U8), InferredTy::Variable(var))
            .unwrap();
        assert_eq!(inferrer.resolve(var), Some(Ty::U8));
    }

    #[test]
    fn unify_two_variables_links_them_transitively() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        inferrer
            .unify(InferredTy::Variable(a), InferredTy::Variable(b))
            .unwrap();
        inferrer
            .unify(InferredTy::Variable(b), InferredTy::Concrete(Ty::U16))
            .unwrap();
        assert_eq!(inferrer.resolve(a), Some(Ty::U16));
        assert_eq!(inferrer.resolve(b), Some(Ty::U16));
    }

    #[test]
    fn resolve_unbound_variable_returns_none() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        assert_eq!(inferrer.resolve(var), None);
    }

    #[test]
    fn bind_prunes_the_bound_value_first() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        inferrer.bind(a, InferredTy::Concrete(Ty::U8));
        inferrer.bind(b, InferredTy::Variable(a));
        assert_eq!(inferrer.resolve(b), Some(Ty::U8));
    }

    #[test]
    fn prune_resolves_chained_variables_to_concrete_ty() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        let c = inferrer.fresh();
        inferrer.bind(a, InferredTy::Variable(b));
        inferrer.bind(b, InferredTy::Variable(c));
        inferrer.bind(c, InferredTy::Concrete(Ty::U4));

        let mut ty = InferredTy::Variable(a);
        inferrer.prune(&mut ty);
        assert_eq!(ty, InferredTy::Concrete(Ty::U4));
    }

    #[test]
    fn prune_on_concrete_ty_is_a_no_op() {
        let mut inferrer = Inferrer::default();
        let mut ty = InferredTy::Concrete(Ty::U8);
        inferrer.prune(&mut ty);
        assert_eq!(ty, InferredTy::Concrete(Ty::U8));
    }

    #[test]
    fn prune_on_unbound_variable_leaves_it_as_a_variable() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        let mut ty = InferredTy::Variable(var);
        inferrer.prune(&mut ty);
        assert_eq!(ty, InferredTy::Variable(var));
    }

    #[test]
    #[should_panic(expected = "does not syntactically support cyclic type definitions")]
    fn bind_panics_on_self_reference() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        inferrer.bind(a, InferredTy::Variable(a));
    }
}
