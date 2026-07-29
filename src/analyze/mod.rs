use crate::analyze::error::*;
use crate::analyze::infer::{InferredTy, Inferrer, TyVar, UnificationError};
use crate::ast::{
    self, BinaryOpKind, FieldPathPart, Identifier, SpannedIdentifier, TyRefData, UnaryOpKind,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::ty::{Ty, TyRegistry};
use miette::SourceSpan;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};

pub mod error;
pub mod infer;

#[derive(Default)]
pub struct Analyzer {
    ty_registry: TyRegistry,
    result_builder: MultiResultBuilder,
}

impl Analyzer {
    pub fn analyze(&mut self, root: &ast::Root) -> Result<(), MultiError> {
        self.register_types(root)?;
        let (mut scopes, fn_decls, const_decls) = self.define_global_scope(root);
        for (const_decl, ty) in const_decls {
            self.type_check_const_decl(const_decl, ty, &mut scopes);
        }
        for (fn_decl, ty) in fn_decls {
            self.type_check_fn_decl(fn_decl, ty, &mut scopes);
        }
        self.result_builder.take().finish(()).into_result()
    }

    fn register_types(&mut self, root: &ast::Root) -> Result<(), MultiError> {
        for item in &root.items {
            if let ast::ItemData::StructDecl(decl) = &item.data
                && let Err(defined) = self
                    .ty_registry
                    .register_struct(decl.name.clone(), decl.fields.len())
            {
                self.result_builder.add_error(RedefinedError {
                    name: decl.name.data.clone(),
                    defined,
                    redefined: decl.name.span,
                });
            }
        }
        for item in &root.items {
            if let ast::ItemData::StructDecl(decl) = &item.data {
                let mut fields = HashMap::new();
                for field in &decl.fields {
                    let ty = self.resolve_ty_ref(&field.ty);
                    match fields.entry(field.name.data.clone()) {
                        Entry::Vacant(entry) => {
                            entry.insert(ty);
                        }
                        Entry::Occupied(_) => {
                            let defined = decl
                                .fields
                                .iter()
                                .find_map(|f| {
                                    if f.name.data == field.name.data {
                                        Some(f.name.span)
                                    } else {
                                        None
                                    }
                                })
                                .unwrap();
                            self.result_builder.add_error(FieldRedefinedError {
                                ty: decl.name.data.clone(),
                                name: field.name.data.clone(),
                                defined,
                                redefined: field.name.span,
                            });
                        }
                    }
                }
                self.ty_registry.register_struct_fields(&decl.name, fields);
            }
        }
        self.result_builder.take().finish(()).into_result()
    }

    fn define_global_scope<'ast>(
        &mut self,
        root: &'ast ast::Root,
    ) -> (
        Scopes,
        Vec<(&'ast ast::FnDecl, Ty)>,
        Vec<(&'ast ast::ConstDecl, Ty)>,
    ) {
        let mut scopes = Scopes::default();
        scopes.enter_global_scope();
        let mut fn_decls = Vec::new();
        let mut const_decls = Vec::new();
        for item in &root.items {
            let (name, ty) = match &item.data {
                ast::ItemData::FnDecl(decl) => {
                    let return_ty = match &decl.signature.ty {
                        None => Ty::UNIT,
                        Some(return_ty) => self.resolve_ty_ref(return_ty),
                    };
                    let fn_ptr_ty = self.ty_registry.register_fn_ptr(return_ty);
                    fn_decls.push((decl, fn_ptr_ty));
                    (&decl.signature.name, InferredTy::Concrete(fn_ptr_ty))
                }
                ast::ItemData::StructDecl(_) => continue,
                ast::ItemData::GlobalDecl(decl) => (
                    &decl.name,
                    InferredTy::Concrete(self.resolve_ty_ref(&decl.ty)),
                ),
                ast::ItemData::ConstDecl(decl) => {
                    let const_ty = self.resolve_ty_ref(&decl.ty);
                    const_decls.push((decl, const_ty));
                    (&decl.name, InferredTy::Concrete(const_ty))
                }
            };
            if let Some(existing) = scopes.insert_place(name.clone(), ty) {
                self.result_builder.add_error(RedefinedError {
                    name: existing.name.data,
                    defined: existing.name.span,
                    redefined: name.span,
                });
            }
        }
        scopes.freeze_global_scope();
        (scopes, fn_decls, const_decls)
    }

    fn resolve_ty_ref(&mut self, ty_ref: &ast::TyRef) -> Ty {
        match &ty_ref.data {
            TyRefData::Simple(name) => self.ty_registry.resolve(&name.data).unwrap_or_else(|| {
                self.result_builder
                    .add_error(TypeNotFoundError::from(name.clone()));
                Ty::ERROR
            }),
            TyRefData::Array(item_ty_ref, count) => {
                let item_ty = self.resolve_ty_ref(&item_ty_ref);
                self.ty_registry.register_array(item_ty, *count)
            }
        }
    }

    fn type_check_const_decl(
        &mut self,
        const_decl: &ast::ConstDecl,
        const_ty: Ty,
        scopes: &mut Scopes,
    ) {
        let mut inferrer = Inferrer::default();
        let mut constraints = VecDeque::new();

        let inferred_ty =
            self.infer_expr(&const_decl.value, &mut inferrer, &mut constraints, scopes);
        constraints.push_back(InferenceConstraint::Equal {
            ty1: inferred_ty,
            ty1_span: const_decl.value.span,
            ty2: InferredTy::Concrete(const_ty),
            ty2_span: const_decl.ty.span,
        });
        self.process_constraints(&mut inferrer, constraints);
    }

    fn type_check_fn_decl(&mut self, fn_decl: &ast::FnDecl, fn_ptr_ty: Ty, scopes: &mut Scopes) {
        let return_ty = self.ty_registry.fn_ptr_ty_data(fn_ptr_ty).return_ty;
        scopes.enter_fn_scope(
            InferredTy::Concrete(return_ty),
            fn_decl
                .signature
                .ty
                .as_ref()
                .map(|ty_ref| ty_ref.span)
                .unwrap_or(fn_decl.signature.name.span),
        );

        let mut inferrer = Inferrer::default();
        let mut constraints = VecDeque::new();

        let inferred_ty = self.infer_block(&fn_decl.body, &mut inferrer, &mut constraints, scopes);
        match inferred_ty {
            InferredTy::Concrete(Ty::NEVER) => {
                // Either it is an infinite loop, or it returns explicitly
                // no constraints to add
            }
            inferred_ty => {
                constraints.push_back(InferenceConstraint::Equal {
                    ty1: inferred_ty,
                    ty1_span: fn_decl.signature.name.span,
                    ty2: InferredTy::Concrete(Ty::UNIT),
                    ty2_span: fn_decl.signature.name.span,
                });
                constraints.push_back(InferenceConstraint::Equal {
                    ty1: InferredTy::Concrete(return_ty),
                    ty1_span: fn_decl.signature.name.span,
                    ty2: InferredTy::Concrete(Ty::UNIT),
                    ty2_span: fn_decl.signature.name.span,
                });
            }
        }
        scopes.exit_fn_scope();
        self.process_constraints(&mut inferrer, constraints);
    }

    fn infer_block(
        &mut self,
        block: &ast::Block,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> InferredTy {
        let mut never_found = false;
        let mut unreachable_code: Option<SourceSpan> = None;
        for statement in &block.statements {
            if never_found {
                unreachable_code = Some(match unreachable_code {
                    Some(start_span) => SourceSpan::from(
                        start_span.offset()..(statement.span.offset() + statement.span.len()),
                    ),
                    None => statement.span,
                });
            }
            let statement_ty = self.infer_statement(statement, inferrer, constraints, scopes);
            if let InferredTy::Concrete(Ty::NEVER) = &statement_ty {
                never_found = true;
            }
        }
        if let Some(span) = unreachable_code {
            self.result_builder.add_error(UnreachableError { span });
        }
        if never_found {
            InferredTy::Concrete(Ty::NEVER)
        } else {
            InferredTy::Concrete(Ty::UNIT)
        }
    }

    fn infer_statement(
        &mut self,
        statement: &ast::Statement,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> InferredTy {
        match &statement.data {
            ast::StatementData::Expr(stmt) => {
                self.infer_expr(&stmt.value, inferrer, constraints, scopes)
            }
            ast::StatementData::Let(stmt) => {
                let let_ty = match &stmt.ty {
                    Some(ty_ref) => InferredTy::Concrete(self.resolve_ty_ref(ty_ref)),
                    None => InferredTy::Variable(inferrer.fresh()),
                };
                let expr_ty = self.infer_expr(&stmt.value, inferrer, constraints, scopes);
                constraints.push_back(InferenceConstraint::Equal {
                    ty1: let_ty.clone(),
                    ty1_span: stmt
                        .ty
                        .as_ref()
                        .map(|ty_ref| ty_ref.span)
                        .unwrap_or(statement.span),
                    ty2: expr_ty,
                    ty2_span: stmt.value.span,
                });
                // ignored the result to allow for local variable shadowing
                let _ = scopes.insert_place(stmt.name.clone(), let_ty);
                InferredTy::Concrete(Ty::UNIT)
            }
            ast::StatementData::Return(stmt) => {
                match scopes.fn_scope() {
                    None => self.result_builder.add_error(ReturnOutsideFnError {
                        span: statement.span,
                    }),
                    Some(fn_scope) => {
                        let return_ty = fn_scope.return_ty.clone();
                        let return_span = fn_scope.span;
                        let return_expr_ty = match &stmt.value {
                            Some(value) => self.infer_expr(value, inferrer, constraints, scopes),
                            None => InferredTy::Concrete(Ty::UNIT),
                        };
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: return_ty,
                            ty1_span: return_span,
                            ty2: return_expr_ty,
                            ty2_span: stmt
                                .value
                                .as_ref()
                                .map(|expr| expr.span)
                                .unwrap_or(statement.span),
                        });
                    }
                }
                InferredTy::Concrete(Ty::NEVER)
            }
            ast::StatementData::Break(stmt) => {
                match scopes.loop_scope() {
                    None => self.result_builder.add_error(BreakOutsideLoopError {
                        span: statement.span,
                    }),
                    Some(loop_scope) => {
                        if loop_scope.break_ty.is_none() {
                            loop_scope.break_ty = Some(inferrer.fresh());
                        }
                        let break_ty = InferredTy::Variable(loop_scope.break_ty.unwrap());
                        let break_span = loop_scope.span;
                        let break_expr_ty = match &stmt.value {
                            Some(value) => self.infer_expr(value, inferrer, constraints, scopes),
                            None => InferredTy::Concrete(Ty::UNIT),
                        };
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: break_ty,
                            ty1_span: break_span,
                            ty2: break_expr_ty,
                            ty2_span: stmt
                                .value
                                .as_ref()
                                .map(|expr| expr.span)
                                .unwrap_or(break_span),
                        });
                    }
                }
                InferredTy::Concrete(Ty::NEVER)
            }
            ast::StatementData::Continue(_) => {
                if scopes.loop_scope().is_none() {
                    self.result_builder.add_error(ContinueOutsideLoopError {
                        span: statement.span,
                    });
                }
                InferredTy::Concrete(Ty::NEVER)
            }
            ast::StatementData::Yield(stmt) => {
                match scopes.block_scope() {
                    Some(block_scope) => {
                        if block_scope.yield_ty.is_none() {
                            block_scope.yield_ty = Some(inferrer.fresh());
                        }
                        let yield_ty = InferredTy::Variable(block_scope.yield_ty.unwrap());
                        let yield_span = block_scope.span;
                        let yield_expr_ty =
                            self.infer_expr(&stmt.value, inferrer, constraints, scopes);
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: yield_ty,
                            ty1_span: yield_span,
                            ty2: yield_expr_ty,
                            ty2_span: stmt.value.span,
                        });
                    }
                    None => {
                        self.result_builder.add_error(YieldOutsideBlockError {
                            span: statement.span,
                        });
                    }
                }
                InferredTy::Concrete(Ty::NEVER)
            }
        }
    }

    fn infer_expr(
        &mut self,
        expr: &ast::Expr,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> InferredTy {
        match &expr.data {
            ast::ExprData::Place(place) => self.infer_place(place, inferrer, constraints, scopes),
            ast::ExprData::Integer(_) => {
                let int_ty = InferredTy::Variable(inferrer.fresh());
                constraints.push_back(InferenceConstraint::IsInt {
                    ty: int_ty.clone(),
                    span: expr.span,
                });
                int_ty
            }
            ast::ExprData::Unary(op) => {
                let expr_ty = self.infer_expr(&op.expr, inferrer, constraints, scopes);
                match op.kind {
                    UnaryOpKind::Not => {
                        constraints.push_back(InferenceConstraint::IsInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                    }
                    UnaryOpKind::Neg => {
                        constraints.push_back(InferenceConstraint::IsSignedInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                    }
                    UnaryOpKind::Deref => todo!("deref"),
                }
                expr_ty
            }
            ast::ExprData::Binary(op) => {
                let left_ty = self.infer_expr(&op.left, inferrer, constraints, scopes);
                let right_ty = self.infer_expr(&op.right, inferrer, constraints, scopes);
                let expr_ty = match op.kind {
                    BinaryOpKind::And
                    | BinaryOpKind::Or
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Le
                    | BinaryOpKind::Ge
                    | BinaryOpKind::Gt
                    | BinaryOpKind::Equal
                    | BinaryOpKind::NotEqual => InferredTy::Concrete(Ty::U1),
                    BinaryOpKind::RotateLeft
                    | BinaryOpKind::RotateRight
                    | BinaryOpKind::ShiftLeft
                    | BinaryOpKind::ShiftRight
                    | BinaryOpKind::ShiftLeftAssign
                    | BinaryOpKind::ShiftRightAssign
                    | BinaryOpKind::RotateLeftAssign
                    | BinaryOpKind::RotateRightAssign => left_ty.clone(),
                    _ => InferredTy::Variable(inferrer.fresh()),
                };
                match op.kind {
                    BinaryOpKind::And | BinaryOpKind::Or => {
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: left_ty.clone(),
                            ty1_span: op.left.span,
                            ty2: InferredTy::Concrete(Ty::U1),
                            ty2_span: expr.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: right_ty.clone(),
                            ty1_span: op.right.span,
                            ty2: InferredTy::Concrete(Ty::U1),
                            ty2_span: expr.span,
                        });
                    }
                    BinaryOpKind::Lt
                    | BinaryOpKind::Le
                    | BinaryOpKind::Gt
                    | BinaryOpKind::Ge
                    | BinaryOpKind::Equal
                    | BinaryOpKind::NotEqual => {
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: left_ty,
                            ty1_span: op.left.span,
                            ty2: right_ty,
                            ty2_span: op.right.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: InferredTy::Concrete(Ty::U1),
                            ty2_span: expr.span,
                        });
                    }
                    BinaryOpKind::Assign
                    | BinaryOpKind::BitAnd
                    | BinaryOpKind::BitOr
                    | BinaryOpKind::Xor
                    | BinaryOpKind::Add
                    | BinaryOpKind::Sub
                    | BinaryOpKind::AddAssign
                    | BinaryOpKind::SubAssign
                    | BinaryOpKind::BitAndAssign
                    | BinaryOpKind::BitOrAssign
                    | BinaryOpKind::XOrAssign => {
                        // currently, assign-like operands assert that the LHS is an int
                        // to ensure that it can fit in a register. Eventually we need
                        // to calculate size of types and assert it can fit in 1 or 2 bytes
                        constraints.push_back(InferenceConstraint::IsInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: left_ty.clone(),
                            ty2_span: op.left.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: right_ty.clone(),
                            ty2_span: op.right.span,
                        });
                    }
                    BinaryOpKind::ShiftLeft
                    | BinaryOpKind::ShiftRight
                    | BinaryOpKind::ShiftLeftAssign
                    | BinaryOpKind::ShiftRightAssign => {
                        constraints.push_back(InferenceConstraint::IsInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: left_ty.clone(),
                            ty2_span: op.left.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: right_ty.clone(),
                            ty1_span: op.right.span,
                            ty2: InferredTy::Concrete(Ty::U1),
                            ty2_span: expr.span,
                        });
                    }
                    BinaryOpKind::RotateLeft
                    | BinaryOpKind::RotateRight
                    | BinaryOpKind::RotateLeftAssign
                    | BinaryOpKind::RotateRightAssign => {
                        constraints.push_back(InferenceConstraint::IsInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: left_ty.clone(),
                            ty2_span: op.left.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: right_ty.clone(),
                            ty1_span: op.right.span,
                            ty2: InferredTy::Concrete(Ty::U1),
                            ty2_span: expr.span,
                        });
                    }
                }
                expr_ty
            }
            ast::ExprData::Block(block) => {
                scopes.enter_block_scope(expr.span);
                let block_expr_ty = self.infer_block(block, inferrer, constraints, scopes);
                match block_expr_ty {
                    InferredTy::Concrete(Ty::NEVER) => {
                        // Either it is an infinite loop, or it yields explicitly
                        // no constraints to add
                    }
                    block_expr_ty => constraints.push_back(InferenceConstraint::Equal {
                        ty1: block_expr_ty,
                        ty1_span: expr.span,
                        ty2: InferredTy::Concrete(Ty::UNIT),
                        ty2_span: expr.span,
                    }),
                }
                let expr_ty = match scopes.block_scope().unwrap().yield_ty {
                    None => {
                        // no yields, block is a unit expression
                        InferredTy::Concrete(Ty::UNIT)
                    }
                    Some(yield_ty) => InferredTy::Variable(yield_ty),
                };
                scopes.exit_block_scope();
                expr_ty
            }
            ast::ExprData::If(if_expr) => {
                let cond_ty = self.infer_expr(&if_expr.condition, inferrer, constraints, scopes);
                constraints.push_back(InferenceConstraint::Equal {
                    ty1: cond_ty.clone(),
                    ty1_span: if_expr.condition.span,
                    ty2: InferredTy::Concrete(Ty::U1),
                    ty2_span: if_expr.condition.span,
                });
                constraints.push_back(InferenceConstraint::IsInt {
                    ty: cond_ty,
                    span: expr.span,
                });
                let true_block_ty =
                    self.infer_expr(&if_expr.true_block, inferrer, constraints, scopes);
                let false_block_ty = match if_expr.false_block.as_ref() {
                    None => InferredTy::Concrete(Ty::UNIT),
                    Some(false_block) => {
                        self.infer_expr(false_block, inferrer, constraints, scopes)
                    }
                };
                match (true_block_ty, false_block_ty) {
                    (InferredTy::Concrete(Ty::NEVER), ty)
                    | (ty, InferredTy::Concrete(Ty::NEVER)) => ty,
                    (true_block_ty, false_block_ty) => {
                        let expr_ty = InferredTy::Variable(inferrer.fresh());
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: true_block_ty.clone(),
                            ty2_span: if_expr.true_block.span,
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: expr_ty.clone(),
                            ty1_span: expr.span,
                            ty2: false_block_ty.clone(),
                            ty2_span: if_expr
                                .false_block
                                .as_deref()
                                .map(|expr| expr.span)
                                .unwrap_or(expr.span),
                        });
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: true_block_ty,
                            ty1_span: if_expr.true_block.span,
                            ty2: false_block_ty,
                            ty2_span: if_expr
                                .false_block
                                .as_deref()
                                .map(|expr| expr.span)
                                .unwrap_or(expr.span),
                        });
                        expr_ty
                    }
                }
            }
            ast::ExprData::Loop(loop_expr) => {
                scopes.enter_loop_scope(expr.span);
                self.infer_block(&loop_expr.body, inferrer, constraints, scopes);
                let expr_ty = match scopes.loop_scope().unwrap().break_ty {
                    None => {
                        // no break, loop is an infinite loop
                        InferredTy::Concrete(Ty::NEVER)
                    }
                    Some(break_ty) => InferredTy::Variable(break_ty),
                };
                scopes.exit_loop_scope();
                expr_ty
            }
            ast::ExprData::Call(call_expr) => {
                let place_ty = self.infer_place(&call_expr.callee, inferrer, constraints, scopes);

                let return_ty = inferrer.fresh();

                constraints.push_back(InferenceConstraint::IsFnPtr {
                    ty: place_ty,
                    span: expr.span,
                    return_ty,
                });
                InferredTy::Variable(return_ty)
            }
            ast::ExprData::Paren(expr) => self.infer_expr(expr, inferrer, constraints, scopes),
        }
    }

    fn infer_place(
        &mut self,
        place: &ast::Place,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> InferredTy {
        let (mut place_ty, mut place_span) = match scopes.get_place(&place.name.data) {
            Some(place_scope) => (place_scope.ty.clone(), place.name.span),
            None => {
                self.result_builder
                    .add_error(NotDefinedError::from(place.name.clone()));
                return InferredTy::Concrete(Ty::ERROR);
            }
        };
        for part in &place.field {
            let (new_place_ty, new_place_span) = match part {
                FieldPathPart::Field(field) => {
                    let field_ty = inferrer.fresh();
                    constraints.push_back(InferenceConstraint::HasField {
                        ty: place_ty,
                        place_span,
                        field: field.clone(),
                        field_ty,
                    });
                    (InferredTy::Variable(field_ty), field.span)
                }
                FieldPathPart::Index(index) => {
                    let index_expr_ty = self.infer_expr(index, inferrer, constraints, scopes);
                    constraints.push_back(InferenceConstraint::Equal {
                        ty1: index_expr_ty,
                        ty1_span: index.span,
                        ty2: InferredTy::Concrete(Ty::U16),
                        ty2_span: index.span,
                    });
                    let item_ty = inferrer.fresh();
                    constraints.push_back(InferenceConstraint::IsArray {
                        ty: place_ty,
                        span: place_span,
                        item_ty,
                    });
                    (InferredTy::Variable(item_ty), index.span)
                }
            };
            place_ty = new_place_ty;
            place_span = new_place_span;
        }
        place_ty
    }

    fn process_constraints(
        &mut self,
        inferrer: &mut Inferrer,
        mut constraints: VecDeque<InferenceConstraint>,
    ) {
        let mut skipped = 0;
        while let Some(constraint) = constraints.pop_front() {
            let constraint = match constraint {
                InferenceConstraint::Equal {
                    ty1,
                    ty1_span,
                    ty2,
                    ty2_span,
                } => {
                    match inferrer.unify(ty1, ty2) {
                        Ok(()) => {}
                        Err(UnificationError {
                            ty1: InferredTy::Concrete(ty1),
                            ty2: InferredTy::Concrete(ty2),
                        }) => {
                            self.result_builder.add_error(TyMismatchError {
                                ty1: ty1.display(&self.ty_registry).into_detached(),
                                ty1_span,
                                ty2: ty2.display(&self.ty_registry).into_detached(),
                                ty2_span,
                            });
                        }
                        Err(UnificationError { .. }) => {
                            self.result_builder.add_error(TyMismatchError {
                                ty1: Ty::UNRESOLVED.display(&self.ty_registry).into_detached(),
                                ty1_span,
                                ty2: Ty::UNRESOLVED.display(&self.ty_registry).into_detached(),
                                ty2_span,
                            });
                        }
                    }
                    None
                }
                InferenceConstraint::IsInt { mut ty, span } => {
                    inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            if !ty.is_int() && ty != Ty::ERROR {
                                self.result_builder.add_error(NotIntError {
                                    ty: ty.display(&self.ty_registry).into_detached(),
                                    span,
                                });
                            }
                            None
                        }
                        ty => Some(InferenceConstraint::IsInt { ty, span }),
                    }
                }
                InferenceConstraint::IsSignedInt { mut ty, span } => {
                    inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            if !ty.is_signed_int() && ty != Ty::ERROR {
                                self.result_builder.add_error(NotSignedIntError {
                                    ty: ty.display(&self.ty_registry).into_detached(),
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
                    inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_field_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_struct() {
                                self.result_builder.add_error(NotStructError {
                                    span: place_span,
                                    ty: ty.display(&self.ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = self.ty_registry.struct_ty_data(ty);
                                match data.fields.get(&field.data).copied() {
                                    None => {
                                        self.result_builder.add_error(FieldNotDefinedError {
                                            struct_span: data.name.span,
                                            ty: ty.display(&self.ty_registry).into_detached(),
                                            field: field.data,
                                            field_span: field.span,
                                        });
                                        Ty::ERROR
                                    }
                                    Some(ty) => ty,
                                }
                            };
                            inferrer.bind(field_ty, InferredTy::Concrete(concrete_field_ty));
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
                    inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_item_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_array() {
                                self.result_builder.add_error(NotArrayError {
                                    span,
                                    ty: ty.display(&self.ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = self.ty_registry.array_ty_data(ty);
                                data.item_ty
                            };
                            inferrer.bind(item_ty, InferredTy::Concrete(concrete_item_ty));
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
                    inferrer.prune(&mut ty);
                    match ty {
                        InferredTy::Concrete(ty) => {
                            let concrete_return_ty = if ty == Ty::ERROR {
                                Ty::ERROR
                            } else if !ty.is_fn_ptr() {
                                self.result_builder.add_error(NotFnPtrError {
                                    span,
                                    ty: ty.display(&self.ty_registry).into_detached(),
                                });
                                Ty::ERROR
                            } else {
                                let data = self.ty_registry.fn_ptr_ty_data(ty);
                                data.return_ty
                            };
                            inferrer.bind(return_ty, InferredTy::Concrete(concrete_return_ty));
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
                    constraints.push_back(constraint);
                    skipped += 1;
                    if skipped >= constraints.len() {
                        break;
                    }
                }
            }
        }

        if !constraints.is_empty() {
            for constraint in constraints {
                match constraint {
                    InferenceConstraint::Equal {
                        mut ty1,
                        ty1_span,
                        mut ty2,
                        ty2_span,
                    } => {
                        inferrer.prune(&mut ty1);
                        inferrer.prune(&mut ty2);
                        let ty1 = match ty1 {
                            InferredTy::Concrete(ty) => ty,
                            InferredTy::Variable(_) => Ty::UNRESOLVED,
                        };
                        let ty2 = match ty2 {
                            InferredTy::Concrete(ty) => ty,
                            InferredTy::Variable(_) => Ty::UNRESOLVED,
                        };
                        self.result_builder.add_error(TyMismatchError {
                            ty1: ty1.display(&self.ty_registry).into_detached(),
                            ty1_span,
                            ty2: ty2.display(&self.ty_registry).into_detached(),
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
    }
}

#[derive(Default)]
struct Scopes {
    fn_scope: Option<FnScope>,
    place_scopes: Vec<HashMap<Identifier, PlaceScope>>,
    global_frozen: bool,
    block_scopes: Vec<Option<BlockScope>>,
    loop_scopes: Vec<LoopScope>,
}

impl Scopes {
    fn get_place(&self, name: &Identifier) -> Option<&PlaceScope> {
        for place_scopes in self.place_scopes.iter().rev() {
            if let Some(place_scope) = place_scopes.get(name) {
                return Some(place_scope);
            }
        }
        None
    }

    #[must_use]
    fn insert_place(&mut self, name: SpannedIdentifier, ty: InferredTy) -> Option<PlaceScope> {
        if self.global_frozen {
            assert!(self.place_scopes.len() > 1, "global scope is frozen");
        } else {
            assert_eq!(self.place_scopes.len(), 1, "not in global scope");
        }
        self.place_scopes
            .last_mut()
            .expect("not in place scope")
            .insert(name.data.clone(), PlaceScope { name, ty })
    }

    fn enter_global_scope(&mut self) {
        assert!(self.place_scopes.is_empty(), "already in global scope");
        self.place_scopes.push(HashMap::new());
    }

    fn freeze_global_scope(&mut self) {
        assert_eq!(self.place_scopes.len(), 1, "not in global scope");
        assert!(!self.global_frozen, "already frozen global scope");
        self.global_frozen = true;
    }

    fn block_scope(&mut self) -> Option<&mut BlockScope> {
        self.block_scopes.last_mut()?.as_mut()
    }

    fn enter_block_scope(&mut self, span: SourceSpan) {
        assert!(!self.place_scopes.is_empty(), "not in global scope");
        self.block_scopes.push(Some(BlockScope {
            span,
            yield_ty: None,
        }));
        self.place_scopes.push(HashMap::new());
    }

    fn exit_block_scope(&mut self) {
        assert!(
            self.place_scopes.len() > 1 && !self.block_scopes.is_empty(),
            "not in block scope"
        );
        let option = self.place_scopes.pop();
        debug_assert!(option.is_some(), "not in place scope");
        let option = self.block_scopes.pop();
        debug_assert!(
            option.is_some() && option.unwrap().is_some(),
            "not in block scope"
        );
    }

    fn loop_scope(&mut self) -> Option<&mut LoopScope> {
        self.loop_scopes.last_mut()
    }

    fn enter_loop_scope(&mut self, span: SourceSpan) {
        self.loop_scopes.push(LoopScope {
            span,
            break_ty: None,
        });
        self.block_scopes.push(None);
    }

    fn exit_loop_scope(&mut self) {
        assert!(!self.loop_scopes.is_empty(), "not in loop scope");
        let option = self.loop_scopes.pop();
        debug_assert!(option.is_some(), "not in loop scope");
        let option = self.block_scopes.pop();
        debug_assert!(option.is_some(), "not in loop scope");
    }

    fn fn_scope(&mut self) -> Option<&mut FnScope> {
        self.fn_scope.as_mut()
    }

    fn enter_fn_scope(&mut self, return_ty: InferredTy, span: SourceSpan) {
        assert!(self.fn_scope.is_none(), "already in fn scope");
        assert_eq!(self.place_scopes.len(), 1, "missing global place scope");
        self.fn_scope = Some(FnScope { return_ty, span });
        self.place_scopes.push(HashMap::new());
        self.block_scopes.push(None);
    }

    fn exit_fn_scope(&mut self) {
        assert!(self.fn_scope.is_some(), "not in fn scope");
        assert_eq!(self.block_scopes.len(), 1, "unbalanced block enters/exits");
        assert!(self.loop_scopes.is_empty(), "unbalanced loop enters/exits");
        assert_eq!(self.place_scopes.len(), 2, "unbalanced place enters/exits");
        self.fn_scope = None;
        let option = self.place_scopes.pop();
        debug_assert!(option.is_some(), "not in place scope");
        let option = self.block_scopes.pop();
        debug_assert!(
            option.is_some() && option.unwrap().is_none(),
            "not in function scope"
        );
    }
}

struct PlaceScope {
    name: SpannedIdentifier,
    ty: InferredTy,
}

struct FnScope {
    return_ty: InferredTy,
    span: SourceSpan,
}

struct BlockScope {
    yield_ty: Option<TyVar>,
    span: SourceSpan,
}

struct LoopScope {
    break_ty: Option<TyVar>,
    span: SourceSpan,
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
