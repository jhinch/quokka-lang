use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use std::fmt;
use std::num::NonZeroU16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredTy {
    Variable(TyVar),
    Concrete(Ty),
    Array(Box<InferredTy>, NonZeroU16),
}

impl InferredTy {
    #[must_use]
    pub fn display<'a>(&'a self, registry: &'a TyRegistry) -> InferredTyDisplay<'a> {
        InferredTyDisplay {
            data: InferredTyDisplayData::Deferred(registry, self),
        }
    }
}

pub struct InferredTyDisplay<'a> {
    data: InferredTyDisplayData<'a>,
}

enum InferredTyDisplayData<'a> {
    Deferred(&'a TyRegistry, &'a InferredTy),
    Rendered(String),
}

impl InferredTyDisplay<'_> {
    #[must_use]
    pub fn into_detached(self) -> InferredTyDisplay<'static> {
        let rendered = match self.data {
            InferredTyDisplayData::Rendered(rendered) => rendered,
            data => format!("{}", InferredTyDisplay { data }),
        };
        InferredTyDisplay {
            data: InferredTyDisplayData::Rendered(rendered),
        }
    }
}

impl fmt::Display for InferredTyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            InferredTyDisplayData::Deferred(registry, ty) => match ty {
                InferredTy::Variable(_) => write!(f, "{}", Ty::UNRESOLVED.display(registry)),
                InferredTy::Concrete(ty) => write!(f, "{}", ty.display(registry)),
                InferredTy::Array(item_ty, count) => {
                    write!(f, "[{}; {}]", item_ty.display(registry), count)
                }
            },
            InferredTyDisplayData::Rendered(rendered) => f.write_str(rendered),
        }
    }
}

impl fmt::Debug for InferredTyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            InferredTyDisplayData::Deferred(_, ty) => write!(f, "{ty:?}"),
            InferredTyDisplayData::Rendered(rendered) => write!(f, "{rendered}"),
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
        ty_registry: &TyRegistry,
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
            (InferredTy::Array(item_ty1, count1), InferredTy::Array(item_ty2, count2)) => {
                if count1 != count2 {
                    return Err(UnificationError {
                        ty1: InferredTy::Array(item_ty1, count1),
                        ty2: InferredTy::Array(item_ty2, count2),
                    });
                }
                self.unify(*item_ty1, *item_ty2, ty_registry).map_err(
                    |UnificationError {
                         ty1: item_ty1,
                         ty2: item_ty2,
                     }| UnificationError {
                        ty1: InferredTy::Array(Box::new(item_ty1), count1),
                        ty2: InferredTy::Array(Box::new(item_ty2), count2),
                    },
                )
            }
            (InferredTy::Concrete(ty1), InferredTy::Array(item_ty2, count2)) if ty1.is_array() => {
                let ty1_data = ty_registry.array_ty_data(ty1);
                if ty1_data.count != count2 {
                    return Err(UnificationError {
                        ty1: InferredTy::Concrete(ty1),
                        ty2: InferredTy::Array(item_ty2, count2),
                    });
                }
                self.unify(
                    InferredTy::Concrete(ty1_data.item_ty),
                    *item_ty2,
                    ty_registry,
                )
                .map_err(|UnificationError { ty2: item_ty2, .. }| UnificationError {
                    ty1: InferredTy::Concrete(ty1),
                    ty2: InferredTy::Array(Box::new(item_ty2), count2),
                })
            }
            (InferredTy::Array(item_ty1, count1), InferredTy::Concrete(ty2)) if ty2.is_array() => {
                let ty2_data = ty_registry.array_ty_data(ty2);
                if count1 != ty2_data.count {
                    return Err(UnificationError {
                        ty1: InferredTy::Array(item_ty1, count1),
                        ty2: InferredTy::Concrete(ty2),
                    });
                }
                self.unify(
                    *item_ty1,
                    InferredTy::Concrete(ty2_data.item_ty),
                    ty_registry,
                )
                .map_err(|UnificationError { ty1: item_ty1, .. }| UnificationError {
                    ty1: InferredTy::Array(Box::new(item_ty1), count1),
                    ty2: InferredTy::Concrete(ty2),
                })
            }
            (InferredTy::Concrete(ty1), InferredTy::Concrete(ty2)) if ty1 == ty2 => Ok(()),
            (ty1, ty2) => Err(UnificationError { ty1, ty2 }),
        }
    }

    pub fn prune(&mut self, ty: &mut InferredTy) {
        let mut original_var = match ty {
            InferredTy::Variable(var) => *var,
            InferredTy::Array(item_ty, _) => {
                self.prune(item_ty);
                return;
            }
            _ => return,
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
                    Some(InferredTy::Array(item_ty, count)) => {
                        let mut item_ty = item_ty.clone();
                        let count = *count;
                        self.prune(&mut item_ty);
                        InferredTy::Array(item_ty, count)
                    }
                    Some(sub_ty) => sub_ty.clone(),
                };
            }
            Some(InferredTy::Array(item_ty, count)) => {
                let mut item_ty = item_ty.clone();
                let count = *count;
                self.prune(&mut item_ty);
                *ty = InferredTy::Array(item_ty, count);
            }
            Some(sub_ty) => {
                *ty = sub_ty.clone();
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
            InferredTy::Array(item_ty, _) => self.occurs(var, item_ty),
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
    fn unify_equal_concrete_tys_succeeds() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Concrete(Ty::U8),
            InferredTy::Concrete(Ty::U8),
            &ty_registry,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_different_concrete_tys_fails() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Concrete(Ty::U8),
            InferredTy::Concrete(Ty::U16),
            &ty_registry,
        );
        let err = result.unwrap_err();
        assert_eq!(err.ty1, InferredTy::Concrete(Ty::U8));
        assert_eq!(err.ty2, InferredTy::Concrete(Ty::U16));
    }

    #[test]
    fn unify_error_ty_is_a_wildcard() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        assert!(
            inferrer
                .unify(
                    InferredTy::Concrete(Ty::ERROR),
                    InferredTy::Concrete(Ty::U16),
                    &ty_registry,
                )
                .is_ok()
        );
        assert!(
            inferrer
                .unify(
                    InferredTy::Concrete(Ty::U16),
                    InferredTy::Concrete(Ty::ERROR),
                    &ty_registry,
                )
                .is_ok()
        );
    }

    #[test]
    fn unify_never_ty_is_not_a_wildcard() {
        // NEVER is intentionally not special-cased like ERROR: it must match exactly.
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Concrete(Ty::NEVER),
            InferredTy::Concrete(Ty::U16),
            &ty_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_same_variable_with_itself_succeeds_without_binding() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        let result = inferrer.unify(
            InferredTy::Variable(var),
            InferredTy::Variable(var),
            &ty_registry,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_variable_with_concrete_binds_variable() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Variable(var),
                InferredTy::Concrete(Ty::U8),
                &ty_registry,
            )
            .unwrap();
        let mut ty = InferredTy::Variable(var);
        inferrer.prune(&mut ty);
        assert_eq!(ty, InferredTy::Concrete(Ty::U8));
    }

    #[test]
    fn unify_concrete_with_variable_binds_variable() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Concrete(Ty::U8),
                InferredTy::Variable(var),
                &ty_registry,
            )
            .unwrap();
        let mut ty = InferredTy::Variable(var);
        inferrer.prune(&mut ty);
        assert_eq!(ty, InferredTy::Concrete(Ty::U8));
    }

    #[test]
    fn unify_two_variables_links_them_transitively() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Variable(a),
                InferredTy::Variable(b),
                &ty_registry,
            )
            .unwrap();
        inferrer
            .unify(
                InferredTy::Variable(b),
                InferredTy::Concrete(Ty::U16),
                &ty_registry,
            )
            .unwrap();
        let mut a = InferredTy::Variable(a);
        inferrer.prune(&mut a);
        assert_eq!(a, InferredTy::Concrete(Ty::U16));
        let mut b = InferredTy::Variable(b);
        inferrer.prune(&mut b);
        assert_eq!(b, InferredTy::Concrete(Ty::U16));
    }

    #[test]
    fn bind_prunes_the_bound_value_first() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        inferrer.bind(a, InferredTy::Concrete(Ty::U8));
        inferrer.bind(b, InferredTy::Variable(a));
        let mut b = InferredTy::Variable(b);
        inferrer.prune(&mut b);
        assert_eq!(b, InferredTy::Concrete(Ty::U8));
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

    fn count(n: u16) -> NonZeroU16 {
        NonZeroU16::new(n).unwrap()
    }

    #[test]
    fn unify_arrays_with_equal_item_and_count_succeeds() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
            &ty_registry,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_arrays_with_different_counts_fails_without_unifying_items() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        let result = inferrer.unify(
            InferredTy::Array(Box::new(InferredTy::Variable(var)), count(3)),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(4)),
            &ty_registry,
        );
        assert!(result.is_err());
        let mut item = InferredTy::Variable(var);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Variable(var));
    }

    #[test]
    fn unify_arrays_with_same_count_but_mismatched_items_fails() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U16)), count(3)),
            &ty_registry,
        );
        let err = result.unwrap_err();
        assert_eq!(
            err.ty1,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3))
        );
        assert_eq!(
            err.ty2,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U16)), count(3))
        );
    }

    #[test]
    fn unify_arrays_with_variable_items_binds_the_variable() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Array(Box::new(InferredTy::Variable(var)), count(2)),
                InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(2)),
                &ty_registry,
            )
            .unwrap();
        let mut item = InferredTy::Variable(var);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Concrete(Ty::U8));
    }

    #[test]
    fn unify_arrays_links_variable_items_transitively() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Array(Box::new(InferredTy::Variable(a)), count(2)),
                InferredTy::Array(Box::new(InferredTy::Variable(b)), count(2)),
                &ty_registry,
            )
            .unwrap();
        inferrer
            .unify(
                InferredTy::Variable(b),
                InferredTy::Concrete(Ty::U4),
                &ty_registry,
            )
            .unwrap();
        let mut item = InferredTy::Variable(a);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Concrete(Ty::U4));
    }

    #[test]
    fn unify_concrete_array_with_matching_inferred_array_succeeds() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U8, count(3));
        let result = inferrer.unify(
            InferredTy::Concrete(concrete_array),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
            &ty_registry,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_concrete_array_with_mismatched_count_fails() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U8, count(3));
        let result = inferrer.unify(
            InferredTy::Concrete(concrete_array),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(4)),
            &ty_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_concrete_array_with_mismatched_count_does_not_unify_items() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U8, count(3));
        let var = inferrer.fresh();
        let result = inferrer.unify(
            InferredTy::Concrete(concrete_array),
            InferredTy::Array(Box::new(InferredTy::Variable(var)), count(4)),
            &ty_registry,
        );
        assert!(result.is_err());
        let mut item = InferredTy::Variable(var);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Variable(var));
    }

    #[test]
    fn unify_concrete_array_with_mismatched_item_ty_fails() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U8, count(3));
        let result = inferrer.unify(
            InferredTy::Concrete(concrete_array),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U16)), count(3)),
            &ty_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_concrete_array_binds_inferred_array_variable_item() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U8, count(3));
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Concrete(concrete_array),
                InferredTy::Array(Box::new(InferredTy::Variable(var)), count(3)),
                &ty_registry,
            )
            .unwrap();
        let mut item = InferredTy::Variable(var);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Concrete(Ty::U8));
    }

    #[test]
    fn unify_inferred_array_with_concrete_array_is_symmetric() {
        let mut inferrer = Inferrer::default();
        let mut ty_registry = TyRegistry::default();
        let concrete_array = ty_registry.register_array(Ty::U16, count(5));
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Array(Box::new(InferredTy::Variable(var)), count(5)),
                InferredTy::Concrete(concrete_array),
                &ty_registry,
            )
            .unwrap();
        let mut item = InferredTy::Variable(var);
        inferrer.prune(&mut item);
        assert_eq!(item, InferredTy::Concrete(Ty::U16));
    }

    #[test]
    fn unify_non_array_concrete_ty_with_inferred_array_fails() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let result = inferrer.unify(
            InferredTy::Concrete(Ty::U8),
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
            &ty_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_error_ty_is_a_wildcard_against_array() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        assert!(
            inferrer
                .unify(
                    InferredTy::Concrete(Ty::ERROR),
                    InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
                    &ty_registry,
                )
                .is_ok()
        );
    }

    #[test]
    fn unify_variable_with_array_binds_variable_to_array() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let var = inferrer.fresh();
        inferrer
            .unify(
                InferredTy::Variable(var),
                InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3)),
                &ty_registry,
            )
            .unwrap();
        let mut ty = InferredTy::Variable(var);
        inferrer.prune(&mut ty);
        assert_eq!(
            ty,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3))
        );
    }

    #[test]
    fn prune_resolves_nested_variable_inside_array_bound_to_variable() {
        let mut inferrer = Inferrer::default();
        let outer = inferrer.fresh();
        let item = inferrer.fresh();
        inferrer.bind(item, InferredTy::Concrete(Ty::U8));
        inferrer.bind(
            outer,
            InferredTy::Array(Box::new(InferredTy::Variable(item)), count(2)),
        );

        let mut ty = InferredTy::Variable(outer);
        inferrer.prune(&mut ty);
        assert_eq!(
            ty,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(2))
        );
    }

    #[test]
    fn prune_resolves_variable_chain_to_array() {
        let mut inferrer = Inferrer::default();
        let a = inferrer.fresh();
        let b = inferrer.fresh();
        let item = inferrer.fresh();
        inferrer.bind(item, InferredTy::Concrete(Ty::U16));
        inferrer.bind(a, InferredTy::Variable(b));
        inferrer.bind(
            b,
            InferredTy::Array(Box::new(InferredTy::Variable(item)), count(4)),
        );

        let mut ty = InferredTy::Variable(a);
        inferrer.prune(&mut ty);
        assert_eq!(
            ty,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U16)), count(4))
        );
    }

    #[test]
    fn prune_leaves_unbound_nested_variable_inside_array() {
        let mut inferrer = Inferrer::default();
        let outer = inferrer.fresh();
        let item = inferrer.fresh();
        inferrer.bind(
            outer,
            InferredTy::Array(Box::new(InferredTy::Variable(item)), count(2)),
        );

        let mut ty = InferredTy::Variable(outer);
        inferrer.prune(&mut ty);
        assert_eq!(
            ty,
            InferredTy::Array(Box::new(InferredTy::Variable(item)), count(2))
        );
    }

    #[test]
    fn prune_on_array_of_concrete_ty_is_a_no_op() {
        let mut inferrer = Inferrer::default();
        let mut ty = InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(2));
        inferrer.prune(&mut ty);
        assert_eq!(
            ty,
            InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(2))
        );
    }

    #[test]
    #[should_panic(expected = "does not syntactically support cyclic type definitions")]
    fn bind_panics_when_array_directly_contains_the_bound_variable() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        inferrer.bind(
            var,
            InferredTy::Array(Box::new(InferredTy::Variable(var)), count(1)),
        );
    }

    #[test]
    #[should_panic(expected = "does not syntactically support cyclic type definitions")]
    fn bind_panics_when_var_occurs_in_nested_array_item() {
        let mut inferrer = Inferrer::default();
        let var = inferrer.fresh();
        let inner = InferredTy::Array(Box::new(InferredTy::Variable(var)), count(2));
        inferrer.bind(var, InferredTy::Array(Box::new(inner), count(3)));
    }

    #[test]
    fn display_of_inferred_array_shows_item_and_count() {
        let registry = TyRegistry::default();
        let ty = InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3));
        assert_eq!(format!("{}", ty.display(&registry)), "[u8; 3]");
    }

    #[test]
    fn display_of_inferred_variable_shows_unresolved_placeholder() {
        let mut inferrer = Inferrer::default();
        let registry = TyRegistry::default();
        let var = inferrer.fresh();
        let ty = InferredTy::Variable(var);
        assert_eq!(
            format!("{}", ty.display(&registry)),
            format!("{}", Ty::UNRESOLVED.display(&registry))
        );
    }

    #[test]
    fn display_of_nested_array_recurses() {
        let mut registry = TyRegistry::default();
        let inner_concrete = registry.register_array(Ty::U8, count(2));
        let ty = InferredTy::Array(Box::new(InferredTy::Concrete(inner_concrete)), count(3));
        assert_eq!(format!("{}", ty.display(&registry)), "[[u8; 2]; 3]");
    }

    #[test]
    fn into_detached_preserves_array_rendering_without_registry() {
        let registry = TyRegistry::default();
        let ty = InferredTy::Array(Box::new(InferredTy::Concrete(Ty::U8)), count(3));
        let detached = ty.display(&registry).into_detached();
        assert_eq!(format!("{detached}"), "[u8; 3]");
    }

    #[test]
    #[should_panic(expected = "does not syntactically support cyclic type definitions")]
    fn bind_should_panic_on_indirect_cycle_through_bound_variable_in_array() {
        let mut inferrer = Inferrer::default();
        let ty_registry = TyRegistry::default();
        let v = inferrer.fresh();
        let x = inferrer.fresh();
        inferrer.bind(x, InferredTy::Variable(v));
        inferrer
            .unify(
                InferredTy::Variable(v),
                InferredTy::Array(Box::new(InferredTy::Variable(x)), count(1)),
                &ty_registry,
            )
            .unwrap();
    }
}
