use crate::analyze::error::{
    BreakExprUnexpectedError, FieldNotDefinedError, NotArrayError, NotFnPtrError, NotIntError,
    NotSignedIntError, NotStructError, ReturnExprMissingError, ReturnExprUnexpectedError,
    TyMismatchError, TypeUnresolvedError, YieldUnexpectedError,
};
use crate::analyze::infer::{InferredTy, Inferrer, TyVar, UnificationError};
use crate::ast::{
    ArithmeticOp, AssignOp, BreakSmt, Call, ComparisonOp, ConstDecl, Expr, If, IntegerLiteral,
    LetSmt, LogicalOp, ReturnSmt, SpannedIdentifier, UnaryOp, UnaryOpKind, YieldSmt,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::ty::{Ty, TyRegistry};
use miette::SourceSpan;
use std::collections::VecDeque;

#[derive(Default)]
pub struct TypeSolver {
    constraints: VecDeque<InferenceConstraint>,
    inferrer: Inferrer,
    result_builder: MultiResultBuilder,
}

impl TypeSolver {
    pub fn fresh_ty_var(&mut self) -> TyVar {
        self.inferrer.fresh()
    }

    pub fn check_const_decl(&mut self, const_decl: &ConstDecl, expr_ty: InferredTy, const_ty: Ty) {
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: expr_ty,
            ty1_span: const_decl.value.span,
            ty2: InferredTy::Concrete(const_ty),
            ty2_span: const_decl.ty.span,
        });
    }

    pub fn check_let_stmt(
        &mut self,
        stmt: &LetSmt,
        expr_span: SourceSpan,
        let_ty: Option<Ty>,
        expr_ty: InferredTy,
    ) -> InferredTy {
        let let_ty = match let_ty {
            Some(let_ty) => InferredTy::Concrete(let_ty),
            None => InferredTy::Variable(self.fresh_ty_var()),
        };
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: let_ty.clone(),
            ty1_span: stmt
                .ty
                .as_ref()
                .map(|ty_ref| ty_ref.span)
                .unwrap_or(expr_span),
            ty2: expr_ty,
            ty2_span: stmt.value.span,
        });
        let_ty
    }

    pub fn check_return_stmt(
        &mut self,
        stmt: &ReturnSmt,
        function_span: SourceSpan,
        statement_span: SourceSpan,
        return_ty: Ty,
        return_expr_ty: Option<InferredTy>,
    ) {
        match (return_ty, return_expr_ty) {
            (Ty::UNIT, Some(_)) => self.result_builder.add_error(ReturnExprUnexpectedError {
                function_span,
                return_span: stmt
                    .value
                    .as_ref()
                    .map(|expr| expr.span)
                    .unwrap_or(statement_span),
            }),
            (Ty::UNIT, None) => {}
            (_, None) => self.result_builder.add_error(ReturnExprMissingError {
                function_span,
                return_span: stmt
                    .value
                    .as_ref()
                    .map(|expr| expr.span)
                    .unwrap_or(statement_span),
            }),
            (return_ty, Some(return_expr_ty)) => {
                self.constraints.push_back(InferenceConstraint::Equal {
                    ty1: return_expr_ty,
                    ty1_span: stmt
                        .value
                        .as_ref()
                        .map(|expr| expr.span)
                        .unwrap_or(statement_span),
                    ty2: InferredTy::Concrete(return_ty),
                    ty2_span: function_span,
                });
            }
        }
    }

    pub fn check_break_stmt(
        &mut self,
        stmt: &BreakSmt,
        loop_span: SourceSpan,
        statement_span: SourceSpan,
        break_ty: Option<InferredTy>,
        break_expr_ty: Option<InferredTy>,
    ) {
        match (break_ty, break_expr_ty) {
            (None, Some(_)) => self.result_builder.add_error(BreakExprUnexpectedError {
                loop_span,
                break_span: stmt
                    .value
                    .as_ref()
                    .map(|expr| expr.span)
                    .unwrap_or(statement_span),
            }),
            (None, None) => {}
            (Some(break_ty), break_expr_ty) => {
                // If break_expr_ty.is_none() it will be an error, but the
                // type checking error will be better when we know what type
                // the loop is expecting
                self.constraints.push_back(InferenceConstraint::Equal {
                    ty1: break_expr_ty.unwrap_or(InferredTy::Concrete(Ty::UNIT)),
                    ty1_span: stmt
                        .value
                        .as_ref()
                        .map(|expr| expr.span)
                        .unwrap_or(statement_span),
                    ty2: break_ty,
                    ty2_span: loop_span,
                });
            }
        }
    }

    pub fn check_yield_stmt(
        &mut self,
        stmt: &YieldSmt,
        block_span: SourceSpan,
        yield_span: SourceSpan,
        yield_ty: Option<InferredTy>,
        yield_expr_ty: InferredTy,
    ) {
        match yield_ty {
            Some(yield_ty) => self.constraints.push_back(InferenceConstraint::Equal {
                ty1: yield_ty,
                ty1_span: yield_span,
                ty2: yield_expr_ty,
                ty2_span: stmt.value.span,
            }),
            None => self.result_builder.add_error(YieldUnexpectedError {
                yield_span,
                block_span,
            }),
        }
    }

    pub fn check_if_expr_condition(&mut self, if_expr: &If, cond_ty: InferredTy) {
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: cond_ty.clone(),
            ty1_span: if_expr.condition.span,
            ty2: InferredTy::Concrete(Ty::U1),
            // this span doesn't make sense. Should be fixed to add an error which
            // is specific to if conditions
            ty2_span: if_expr.condition.span,
        });
    }

    #[must_use]
    pub fn check_integer_expr(&mut self, _: &IntegerLiteral, expr_span: SourceSpan) -> InferredTy {
        let int_ty = InferredTy::Variable(self.fresh_ty_var());
        self.constraints.push_back(InferenceConstraint::IsInt {
            ty: int_ty.clone(),
            span: expr_span,
        });
        int_ty
    }

    #[must_use]
    pub fn check_unary_expr(
        &mut self,
        op: &UnaryOp,
        expr_span: SourceSpan,
        inner_ty: InferredTy,
    ) -> InferredTy {
        match op.kind {
            UnaryOpKind::Neg => {
                self.constraints
                    .push_back(InferenceConstraint::IsSignedInt {
                        ty: inner_ty.clone(),
                        span: expr_span,
                    });
                inner_ty
            }
            UnaryOpKind::Not
            | UnaryOpKind::ShiftLeft
            | UnaryOpKind::ShiftRight
            | UnaryOpKind::RotateLeft
            | UnaryOpKind::RotateRight => {
                self.constraints.push_back(InferenceConstraint::IsInt {
                    ty: inner_ty.clone(),
                    span: expr_span,
                });
                inner_ty
            }
        }
    }

    #[must_use]
    pub fn check_logical_expr(
        &mut self,
        op: &LogicalOp,
        expr_span: SourceSpan,
        left_ty: InferredTy,
        right_ty: InferredTy,
    ) -> InferredTy {
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: left_ty.clone(),
            ty1_span: op.left.span,
            ty2: InferredTy::Concrete(Ty::U1),
            ty2_span: expr_span,
        });
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: right_ty.clone(),
            ty1_span: op.right.span,
            ty2: InferredTy::Concrete(Ty::U1),
            ty2_span: expr_span,
        });
        InferredTy::Concrete(Ty::U1)
    }

    #[must_use]
    pub fn check_arithmetic_expr(
        &mut self,
        op: &ArithmeticOp,
        expr_span: SourceSpan,
        left_ty: InferredTy,
        right_ty: InferredTy,
    ) -> InferredTy {
        let expr_ty = InferredTy::Variable(self.fresh_ty_var());
        self.constraints.push_back(InferenceConstraint::IsInt {
            ty: expr_ty.clone(),
            span: expr_span,
        });
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: expr_ty.clone(),
            ty1_span: expr_span,
            ty2: left_ty.clone(),
            ty2_span: op.left.span,
        });
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: expr_ty.clone(),
            ty1_span: expr_span,
            ty2: right_ty.clone(),
            ty2_span: op.right.span,
        });
        expr_ty
    }

    #[must_use]
    pub fn check_comparison_expr(
        &mut self,
        op: &ComparisonOp,
        _: SourceSpan,
        left_ty: InferredTy,
        right_ty: InferredTy,
    ) -> InferredTy {
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: left_ty,
            ty1_span: op.left.span,
            ty2: right_ty,
            ty2_span: op.right.span,
        });
        InferredTy::Concrete(Ty::U1)
    }

    #[must_use]
    pub fn check_assign_expr(
        &mut self,
        op: &AssignOp,
        expr_span: SourceSpan,
        left_ty: InferredTy,
        right_ty: InferredTy,
    ) -> InferredTy {
        let expr_ty = InferredTy::Variable(self.fresh_ty_var());
        // currently, assign-like operands assert that the LHS is an int
        // to ensure that it can fit in a register. Eventually we need
        // to calculate size of types and assert it can fit in 1 or 2 bytes
        self.constraints.push_back(InferenceConstraint::IsInt {
            ty: expr_ty.clone(),
            span: expr_span,
        });
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: expr_ty.clone(),
            ty1_span: expr_span,
            ty2: left_ty,
            ty2_span: op.left.span,
        });
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: expr_ty.clone(),
            ty1_span: expr_span,
            ty2: right_ty,
            ty2_span: op.right.span,
        });
        expr_ty
    }

    pub fn check_if_expr(
        &mut self,
        if_expr: &If,
        expr_span: SourceSpan,
        then_ty: InferredTy,
        else_ty: Option<InferredTy>,
    ) -> InferredTy {
        let else_ty = else_ty.unwrap_or(InferredTy::Concrete(Ty::UNIT));
        match (then_ty, else_ty) {
            (InferredTy::Concrete(Ty::NEVER), ty) | (ty, InferredTy::Concrete(Ty::NEVER)) => {
                // if one side is divergent, we don't need to add any
                // additional constraints. Simply return the other side
                ty
            }
            (then_ty, else_ty) => {
                let expr_ty = InferredTy::Variable(self.fresh_ty_var());
                self.constraints.push_back(InferenceConstraint::Equal {
                    ty1: expr_ty.clone(),
                    ty1_span: expr_span,
                    ty2: then_ty.clone(),
                    ty2_span: if_expr.true_block.span,
                });
                self.constraints.push_back(InferenceConstraint::Equal {
                    ty1: expr_ty.clone(),
                    ty1_span: expr_span,
                    ty2: else_ty.clone(),
                    ty2_span: if_expr
                        .false_block
                        .as_deref()
                        .map(|expr| expr.span)
                        .unwrap_or(expr_span),
                });
                expr_ty
            }
        }
    }

    pub fn check_call_expr(
        &mut self,
        _: &Call,
        expr_span: SourceSpan,
        callee_ty: InferredTy,
    ) -> InferredTy {
        let return_ty = self.fresh_ty_var();

        self.constraints.push_back(InferenceConstraint::IsFnPtr {
            ty: callee_ty,
            span: expr_span,
            return_ty,
        });
        InferredTy::Variable(return_ty)
    }

    pub fn check_field_access(
        &mut self,
        field: &SpannedIdentifier,
        place_span: SourceSpan,
        place_ty: InferredTy,
    ) -> InferredTy {
        let field_ty = self.fresh_ty_var();
        self.constraints.push_back(InferenceConstraint::HasField {
            ty: place_ty,
            place_span,
            field: field.clone(),
            field_ty,
        });
        InferredTy::Variable(field_ty)
    }

    pub fn check_array_index(
        &mut self,
        index: &Expr,
        place_span: SourceSpan,
        place_ty: InferredTy,
        index_expr_ty: InferredTy,
    ) -> InferredTy {
        self.constraints.push_back(InferenceConstraint::Equal {
            ty1: index_expr_ty,
            ty1_span: index.span,
            ty2: InferredTy::Concrete(Ty::U16),
            ty2_span: index.span,
        });
        let item_ty = self.fresh_ty_var();
        self.constraints.push_back(InferenceConstraint::IsArray {
            ty: place_ty,
            span: place_span,
            item_ty,
        });
        InferredTy::Variable(item_ty)
    }

    pub fn solve(mut self, ty_registry: &TyRegistry) -> Result<(), MultiError> {
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
                            self.result_builder.add_error(TyMismatchError {
                                ty1: ty1.display(ty_registry).into_detached(),
                                ty1_span,
                                ty2: ty2.display(ty_registry).into_detached(),
                                ty2_span,
                            });
                        }
                        Err(UnificationError { .. }) => {
                            self.result_builder.add_error(TyMismatchError {
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
                                self.result_builder.add_error(NotIntError {
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
                                self.result_builder.add_error(NotSignedIntError {
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
                                self.result_builder.add_error(NotStructError {
                                    span: place_span,
                                    ty: ty.display(ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = ty_registry.struct_ty_data(ty);
                                match data.fields.get(&field.data) {
                                    None => {
                                        self.result_builder.add_error(FieldNotDefinedError {
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
                                self.result_builder.add_error(NotArrayError {
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
                                self.result_builder.add_error(NotFnPtrError {
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
                        self.result_builder.add_error(TyMismatchError {
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
                        self.result_builder.add_error(TypeUnresolvedError { span });
                    }
                    InferenceConstraint::HasField { field, .. } => {
                        self.result_builder
                            .add_error(TypeUnresolvedError { span: field.span });
                    }
                }
            }
        }
        self.result_builder.finish(()).into_result()
    }
}

#[derive(Debug)]
enum InferenceConstraint {
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
