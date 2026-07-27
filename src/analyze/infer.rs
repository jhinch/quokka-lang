use crate::ty::Ty;
use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredTy {
    Variable(TyVar),
    Concrete(Ty),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVar(u32);

#[derive(Default)]
pub struct Inferrer {
    substitutions: IndexMap<TyVar, InferredTy>,
    next_ty_var: u32,
}

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
        if self.occurs(var, &other) {
            panic!(
                "{var:?} occurs in {other:?} however quokka does not syntactically support cyclic type definitions"
            );
        }
        self.substitutions.insert(var, other);
    }

    fn occurs(&self, var: TyVar, other: &InferredTy) -> bool {
        match other {
            InferredTy::Variable(v) => *v == var,
            InferredTy::Concrete(_) => false,
        }
    }
}
