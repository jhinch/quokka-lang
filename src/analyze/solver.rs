use crate::analyze::error::{
    FieldNotDefinedError, NotArrayError, NotFnPtrError, NotIntError, NotSignedIntError,
    NotStructError, TyMismatchError, TypeUnresolvedError,
};
use crate::analyze::infer::{InferredTy, Inferrer, TyVar, UnificationError};
use crate::ast::SpannedIdentifier;
use crate::multi::{MultiError, MultiResultBuilder};
use crate::ty::{Ty, TyRegistry};
use miette::SourceSpan;
use std::collections::VecDeque;

#[derive(Default)]
pub struct TypeSolver {
    constraints: VecDeque<InferenceConstraint>,
    inferrer: Inferrer,
}

impl TypeSolver {
    pub fn fresh_ty_var(&mut self) -> TyVar {
        self.inferrer.fresh()
    }

    pub fn insert_constraint(&mut self, constraint: InferenceConstraint) {
        self.constraints.push_back(constraint);
    }

    pub fn solve(mut self, ty_registry: &TyRegistry) -> Result<(), MultiError> {
        let mut result_builder = MultiResultBuilder::default();
        let mut skipped = 0;
        while let Some(constraint) = self.constraints.pop_front() {
            let constraint = match constraint {
                InferenceConstraint::Equal {
                    ty1,
                    ty1_span,
                    ty2,
                    ty2_span,
                } => {
                    match self.inferrer.unify(ty1, ty2) {
                        Ok(()) => {}
                        Err(UnificationError {
                            ty1: InferredTy::Concrete(ty1),
                            ty2: InferredTy::Concrete(ty2),
                        }) => {
                            result_builder.add_error(TyMismatchError {
                                ty1: ty1.display(ty_registry).into_detached(),
                                ty1_span,
                                ty2: ty2.display(ty_registry).into_detached(),
                                ty2_span,
                            });
                        }
                        Err(UnificationError { .. }) => {
                            result_builder.add_error(TyMismatchError {
                                ty1: Ty::UNRESOLVED.display(ty_registry).into_detached(),
                                ty1_span,
                                ty2: Ty::UNRESOLVED.display(ty_registry).into_detached(),
                                ty2_span,
                            });
                        }
                    }
                    None
                }
                InferenceConstraint::IsInt { mut ty, span } => {
                    self.inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            if !ty.is_int() && ty != Ty::ERROR {
                                result_builder.add_error(NotIntError {
                                    ty: ty.display(ty_registry).into_detached(),
                                    span,
                                });
                            }
                            None
                        }
                        ty => Some(InferenceConstraint::IsInt { ty, span }),
                    }
                }
                InferenceConstraint::IsSignedInt { mut ty, span } => {
                    self.inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            if !ty.is_signed_int() && ty != Ty::ERROR {
                                result_builder.add_error(NotSignedIntError {
                                    ty: ty.display(ty_registry).into_detached(),
                                    span,
                                });
                            }
                            None
                        }
                        ty => Some(InferenceConstraint::IsSignedInt { ty, span }),
                    }
                }
                InferenceConstraint::HasField {
                    mut ty,
                    place_span,
                    field,
                    field_ty,
                } => {
                    self.inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_field_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_struct() {
                                result_builder.add_error(NotStructError {
                                    span: place_span,
                                    ty: ty.display(ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = ty_registry.struct_ty_data(ty);
                                match data.fields.get(&field.data) {
                                    None => {
                                        result_builder.add_error(FieldNotDefinedError {
                                            struct_span: data.name.span,
                                            ty: ty.display(ty_registry).into_detached(),
                                            field: field.data,
                                            field_span: field.span,
                                        });
                                        Ty::ERROR
                                    }
                                    Some(field_data) => field_data.ty,
                                }
                            };
                            self.inferrer
                                .bind(field_ty, InferredTy::Concrete(concrete_field_ty));
                            None
                        }
                        ty => Some(InferenceConstraint::HasField {
                            ty,
                            place_span,
                            field,
                            field_ty,
                        }),
                    }
                }
                InferenceConstraint::IsArray {
                    mut ty,
                    span,
                    item_ty,
                } => {
                    self.inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_item_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_array() {
                                result_builder.add_error(NotArrayError {
                                    span,
                                    ty: ty.display(ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = ty_registry.array_ty_data(ty);
                                data.item_ty
                            };
                            self.inferrer
                                .bind(item_ty, InferredTy::Concrete(concrete_item_ty));
                            None
                        }
                        ty => Some(InferenceConstraint::IsArray { ty, span, item_ty }),
                    }
                }
                InferenceConstraint::IsFnPtr {
                    mut ty,
                    span,
                    return_ty,
                } => {
                    self.inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_return_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_fn_ptr() {
                                result_builder.add_error(NotFnPtrError {
                                    span,
                                    ty: ty.display(ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = ty_registry.fn_ptr_ty_data(ty);
                                data.return_ty
                            };
                            self.inferrer
                                .bind(return_ty, InferredTy::Concrete(concrete_return_ty));
                            None
                        }
                        ty => Some(InferenceConstraint::IsFnPtr {
                            ty,
                            span,
                            return_ty,
                        }),
                    }
                }
            };
            match constraint {
                None => {
                    skipped = 0;
                }
                Some(constraint) => {
                    self.constraints.push_back(constraint);
                    skipped += 1;
                    if skipped >= self.constraints.len() {
                        break;
                    }
                }
            }
        }

        if !self.constraints.is_empty() {
            for constraint in self.constraints {
                match constraint {
                    InferenceConstraint::Equal {
                        mut ty1,
                        ty1_span,
                        mut ty2,
                        ty2_span,
                    } => {
                        self.inferrer.prune(&mut ty1);
                        self.inferrer.prune(&mut ty2);
                        let ty1 = match ty1 {
                            InferredTy::Concrete(ty) => ty,
                            InferredTy::Variable(_) => Ty::UNRESOLVED,
                        };
                        let ty2 = match ty2 {
                            InferredTy::Concrete(ty) => ty,
                            InferredTy::Variable(_) => Ty::UNRESOLVED,
                        };
                        result_builder.add_error(TyMismatchError {
                            ty1: ty1.display(ty_registry).into_detached(),
                            ty1_span,
                            ty2: ty2.display(ty_registry).into_detached(),
                            ty2_span,
                        });
                    }
                    InferenceConstraint::IsInt { span, .. }
                    | InferenceConstraint::IsSignedInt { span, .. }
                    | InferenceConstraint::IsArray { span, .. }
                    | InferenceConstraint::IsFnPtr { span, .. } => {
                        result_builder.add_error(TypeUnresolvedError { span });
                    }
                    InferenceConstraint::HasField { field, .. } => {
                        result_builder.add_error(TypeUnresolvedError { span: field.span });
                    }
                }
            }
        }
        result_builder.finish(()).into_result()
    }
}

#[derive(Debug)]
pub enum InferenceConstraint {
    Equal {
        ty1: InferredTy,
        ty1_span: SourceSpan,
        ty2: InferredTy,
        ty2_span: SourceSpan,
    },
    IsInt {
        ty: InferredTy,
        span: SourceSpan,
    },
    IsSignedInt {
        ty: InferredTy,
        span: SourceSpan,
    },
    HasField {
        ty: InferredTy,
        place_span: SourceSpan,
        field: SpannedIdentifier,
        field_ty: TyVar,
    },
    IsArray {
        ty: InferredTy,
        span: SourceSpan,
        item_ty: TyVar,
    },
    IsFnPtr {
        ty: InferredTy,
        span: SourceSpan,
        return_ty: TyVar,
    },
}
