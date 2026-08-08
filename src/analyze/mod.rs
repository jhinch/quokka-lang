use crate::analyze::error::*;
use crate::analyze::infer::{InferredTy, Inferrer, TyVar, UnificationError};
use crate::ast::{
    self, ArithmeticOpKind, AssignOpKind, ComparisonOpKind, Identifier, LogicalOpKind,
    PlaceProjection, SpannedIdentifier, TyRefData, UnaryOpKind,
};
use crate::attr::{
    ConstDeclAttributes, FieldDeclAttributes, FnDeclAttributes, GlobalDeclAttributes,
    StructDeclAttributes,
};
use crate::mir::{
    BlockId, Callee, Condition, FunctionBuilder, FunctionId, GlobalId, GlobalPlace,
    GlobalPlaceProjection, Instr, LocalId, Mir, MirBuilder, Operand, Terminator, Variable,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::source::span::span_between;
use crate::ty::{StructFieldTyData, Ty, TyRegistry};
use indexmap::map::Entry;
use indexmap::{IndexMap, IndexSet};
use miette::SourceSpan;
use std::collections::{HashMap, VecDeque};

pub mod error;
pub mod infer;

#[derive(Default)]
pub struct Analyzer {
    ty_registry: TyRegistry,
    mir_builder: MirBuilder,
    result_builder: MultiResultBuilder,
}

impl Analyzer {
    pub fn analyze(mut self, root: &ast::Root) -> Result<(Mir, TyRegistry), MultiError> {
        self.register_types(root)?;
        let (mut scopes, fn_decls, const_decls) = self.define_global_scope(root);
        for (const_decl, ty, _) in const_decls {
            self.analyze_const_decl(const_decl, ty, &mut scopes);
        }
        for (fn_decl, ty, attrs, function_id) in fn_decls {
            self.analyze_fn_decl(fn_decl, ty, attrs, function_id, &mut scopes);
        }
        self.result_builder.take().finish(()).into_result()?;
        Ok((self.mir_builder.build(), self.ty_registry))
    }

    fn register_types(&mut self, root: &ast::Root) -> Result<(), MultiError> {
        // first register all the structs
        let registered_types = root
            .items
            .iter()
            .filter_map(|item| {
                if let ast::ItemData::StructDecl(decl) = &item.data {
                    let _attrs = StructDeclAttributes::try_from(item.attributes.as_slice())
                        .unwrap_or_else(|error| {
                            self.result_builder.add_error(error);
                            StructDeclAttributes::default()
                        });
                    match self.ty_registry.register_struct(decl.name.clone()) {
                        Ok(ty) => Some((ty, decl)),
                        Err(defined) => {
                            self.result_builder.add_error(RedefinedError {
                                name: decl.name.data.clone(),
                                defined,
                                redefined: decl.name.span,
                            });
                            None
                        }
                    }
                } else {
                    None
                }
            })
            .collect::<IndexMap<_, _>>();
        // then calculate their fields
        let type_fields = registered_types
            .iter()
            .filter_map(|(ty, decl)| {
                let mut fields = IndexMap::with_capacity(decl.fields.len());
                let mut failed = false;
                for field in &decl.fields {
                    let ty = self.resolve_ty_ref(&field.ty);
                    let field_attrs = FieldDeclAttributes::try_from(field.attributes.as_slice())
                        .unwrap_or_else(|e| {
                            self.result_builder.add_error(e);
                            FieldDeclAttributes::default()
                        });
                    match fields.entry(field.name.data.clone()) {
                        Entry::Vacant(entry) => {
                            entry.insert((ty, field.name.span, field.ty.span, field_attrs));
                        }
                        Entry::Occupied(entry) => {
                            let defined = decl.fields[entry.index()].name.span;
                            self.result_builder.add_error(FieldRedefinedError {
                                ty: decl.name.data.clone(),
                                name: field.name.data.clone(),
                                defined,
                                redefined: field.name.span,
                            });
                            failed = true;
                        }
                    }
                }
                if failed { None } else { Some((*ty, fields)) }
            })
            .collect::<IndexMap<_, _>>();
        // finally, calculate the size of each struct, identifying cyclic
        // types as a result
        let mut unprocessed_types = type_fields.keys().rev().copied().collect::<IndexSet<_>>();
        let mut recursive_types = IndexSet::new();
        let mut processing_stack = VecDeque::new();
        while let Some(ty) = unprocessed_types.last().copied() {
            processing_stack.push_front(ty);
            'outer: while let Some(ty) = processing_stack.pop_front() {
                let mut bit_size = 0;
                let mut fields = IndexMap::new();
                for (field_name, (field_ty, field_span, field_ty_span, field_attrs)) in
                    &type_fields[&ty]
                {
                    let mut array_item_ty = *field_ty;
                    while array_item_ty.is_array() {
                        array_item_ty = self.ty_registry.array_ty_data(array_item_ty).item_ty;
                    }
                    if array_item_ty.is_struct() && unprocessed_types.contains(&array_item_ty) {
                        // we need to calculate the field first
                        processing_stack.push_front(ty);
                        if processing_stack.contains(&array_item_ty) {
                            self.result_builder.add_error(TypeRecursiveError::from(
                                registered_types[&array_item_ty].name.clone(),
                            ));
                            while let Some(ty) = processing_stack.pop_front() {
                                unprocessed_types.shift_remove(&ty);
                                recursive_types.insert(ty);
                                if ty == array_item_ty {
                                    break;
                                }
                            }
                            continue 'outer;
                        }
                        processing_stack.push_front(array_item_ty);
                        continue 'outer;
                    } else if recursive_types.contains(&array_item_ty) {
                        unprocessed_types.shift_remove(&ty);
                        recursive_types.insert(ty);
                        continue 'outer;
                    }
                    let field_bit_size = field_ty.bit_size(&self.ty_registry).next_multiple_of(8);
                    fields.insert(
                        field_name.clone(),
                        StructFieldTyData {
                            name: SpannedIdentifier::new(field_name.clone(), *field_span),
                            ty: *field_ty,
                            ty_span: *field_ty_span,
                            bit_offset: bit_size,
                            attrs: field_attrs.clone(),
                        },
                    );
                    bit_size += field_bit_size;
                }
                self.ty_registry
                    .complete_struct_registration(ty, fields, bit_size);
                unprocessed_types.shift_remove(&ty);
            }
        }
        self.result_builder.take().finish(()).into_result()
    }

    fn define_global_scope<'ast>(
        &mut self,
        root: &'ast ast::Root,
    ) -> (
        Scopes,
        Vec<(&'ast ast::FnDecl, Ty, FnDeclAttributes, FunctionId)>,
        Vec<(&'ast ast::ConstDecl, Ty, ConstDeclAttributes)>,
    ) {
        let mut scopes = Scopes::default();
        scopes.enter_global_scope();
        let mut fn_decls = Vec::new();
        let mut const_decls = Vec::new();
        for item in &root.items {
            let (name, ty, mir_id) = match &item.data {
                ast::ItemData::FnDecl(decl) => {
                    let return_ty = match &decl.signature.ty {
                        None => Ty::UNIT,
                        Some(return_ty) => self.resolve_ty_ref(return_ty),
                    };
                    let attrs = match FnDeclAttributes::try_from(item.attributes.as_slice()) {
                        Ok(attrs) => attrs,
                        Err(error) => {
                            self.result_builder.add_error(error);
                            FnDeclAttributes::default()
                        }
                    };
                    let fn_ptr_ty = self.ty_registry.register_fn_ptr(return_ty);
                    let function_id = self.mir_builder.reserve_function();
                    fn_decls.push((decl, fn_ptr_ty, attrs, function_id));
                    (
                        &decl.signature.name,
                        InferredTy::Concrete(fn_ptr_ty),
                        PlaceId::from(function_id),
                    )
                }
                ast::ItemData::StructDecl(_) => continue,
                ast::ItemData::GlobalDecl(decl) => {
                    let ty = self.resolve_ty_ref(&decl.ty);
                    let attrs = match GlobalDeclAttributes::try_from(item.attributes.as_slice()) {
                        Ok(attrs) => attrs,
                        Err(error) => {
                            self.result_builder.add_error(error);
                            GlobalDeclAttributes::default()
                        }
                    };
                    (
                        &decl.name,
                        InferredTy::Concrete(ty),
                        PlaceId::from(self.mir_builder.insert_global(
                            decl.name.clone(),
                            ty,
                            attrs.into(),
                        )),
                    )
                }
                ast::ItemData::ConstDecl(decl) => {
                    let const_ty = self.resolve_ty_ref(&decl.ty);
                    let attrs = match ConstDeclAttributes::try_from(item.attributes.as_slice()) {
                        Ok(attrs) => attrs,
                        Err(error) => {
                            self.result_builder.add_error(error);
                            ConstDeclAttributes::default()
                        }
                    };
                    const_decls.push((decl, const_ty, attrs.clone()));
                    (
                        &decl.name,
                        InferredTy::Concrete(const_ty),
                        PlaceId::from(self.mir_builder.insert_global(
                            decl.name.clone(),
                            const_ty,
                            attrs.into(),
                        )),
                    )
                }
            };
            if let Some(existing) = scopes.insert_place(name.clone(), ty, mir_id) {
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
                let item_ty = self.resolve_ty_ref(item_ty_ref);
                self.ty_registry.register_array(item_ty, *count)
            }
        }
    }

    fn analyze_const_decl(
        &mut self,
        const_decl: &ast::ConstDecl,
        const_ty: Ty,
        scopes: &mut Scopes,
    ) {
        let mut mir = FunctionBuilder::new(const_decl.name.data.clone(), const_ty);
        let mut inferrer = Inferrer::default();
        let mut constraints = VecDeque::new();

        let block = mir.new_block();
        mir.seal_block(block);

        let (inferred_ty, _, _) = self.analyze_expr(
            &const_decl.value,
            block,
            &mut mir,
            &mut inferrer,
            &mut constraints,
            scopes,
        );
        constraints.push_back(InferenceConstraint::Equal {
            ty1: inferred_ty,
            ty1_span: const_decl.value.span,
            ty2: InferredTy::Concrete(const_ty),
            ty2_span: const_decl.ty.span,
        });
        self.process_constraints(&mut inferrer, constraints);
    }

    fn analyze_fn_decl(
        &mut self,
        fn_decl: &ast::FnDecl,
        fn_ptr_ty: Ty,
        attrs: FnDeclAttributes,
        function_id: FunctionId,
        scopes: &mut Scopes,
    ) {
        let return_ty = self.ty_registry.fn_ptr_ty_data(fn_ptr_ty).return_ty;
        let mut mir = FunctionBuilder::new(fn_decl.signature.name.data.clone(), return_ty);
        mir.set_attrs(attrs);
        let return_local = if return_ty != Ty::UNIT && return_ty != Ty::NEVER {
            Some(mir.create_return_local(return_ty))
        } else {
            None
        };
        scopes.enter_fn_scope(
            InferredTy::Concrete(return_ty),
            fn_decl
                .signature
                .ty
                .as_ref()
                .map(|ty_ref| ty_ref.span)
                .unwrap_or(fn_decl.signature.name.span),
            return_local,
        );

        let mut inferrer = Inferrer::default();
        let mut constraints = VecDeque::new();

        let block = mir.new_block();
        mir.seal_block(block);

        let (inferred_ty, block) = self.analyze_block(
            &fn_decl.body,
            block,
            &mut mir,
            &mut inferrer,
            &mut constraints,
            scopes,
        );
        match inferred_ty {
            Ty::NEVER => {
                // Either it is an infinite loop, or it returns explicitly
                // no constraints to add
            }
            inferred_ty => {
                constraints.push_back(InferenceConstraint::Equal {
                    ty1: InferredTy::Concrete(inferred_ty),
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
                mir.set_terminator(block, Terminator::Return);
            }
        }
        scopes.exit_fn_scope();
        self.process_constraints(&mut inferrer, constraints);

        self.mir_builder.seal_function(function_id, mir);
    }

    fn analyze_block(
        &mut self,
        ast_block: &ast::Block,
        mut block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (Ty, BlockId) {
        let mut never_found = false;
        let mut unreachable_code: Option<SourceSpan> = None;
        for statement in &ast_block.statements {
            if never_found {
                unreachable_code = Some(match unreachable_code {
                    Some(start_span) => span_between(start_span, statement.span),
                    None => statement.span,
                });
            }
            let (statement_ty, next_block) =
                self.analyze_statement(statement, block, mir, inferrer, constraints, scopes);
            block = next_block;
            if let InferredTy::Concrete(Ty::NEVER) = &statement_ty {
                never_found = true;
            }
        }
        if let Some(span) = unreachable_code {
            self.result_builder.add_error(UnreachableError { span });
        }
        let ty = if never_found { Ty::NEVER } else { Ty::UNIT };
        (ty, block)
    }

    fn analyze_statement(
        &mut self,
        statement: &ast::Statement,
        block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (InferredTy, BlockId) {
        match &statement.data {
            ast::StatementData::Expr(stmt) => {
                let (ty, block) =
                    self.analyze_expr_unit(&stmt.value, block, mir, inferrer, constraints, scopes);
                (ty, block)
            }
            ast::StatementData::Let(stmt) => {
                let let_ty = match &stmt.ty {
                    Some(ty_ref) => InferredTy::Concrete(self.resolve_ty_ref(ty_ref)),
                    None => InferredTy::Variable(inferrer.fresh()),
                };
                let (expr_ty, block, source) =
                    self.analyze_expr(&stmt.value, block, mir, inferrer, constraints, scopes);
                let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
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
                let variable = mir.new_variable();
                mir.push_instr(block, Instr::Copy { target, source });
                mir.set_variable_name(variable, stmt.name.clone());
                mir.write_variable(variable, block, target);
                // ignored the result to allow for local variable shadowing
                let _ = scopes.insert_place(stmt.name.clone(), let_ty, PlaceId::from(variable));
                (InferredTy::Concrete(Ty::UNIT), block)
            }
            ast::StatementData::Return(stmt) => {
                match scopes.fn_scope() {
                    None => self.result_builder.add_error(ReturnOutsideFnError {
                        span: statement.span,
                    }),
                    Some(fn_scope) => {
                        let return_ty = fn_scope.return_ty.clone();
                        let return_span = fn_scope.span;
                        let (return_expr_ty, block) = match &stmt.value {
                            Some(value) => {
                                let return_local = fn_scope.return_local;
                                let (ty, block, source) = self.analyze_expr(
                                    value,
                                    block,
                                    mir,
                                    inferrer,
                                    constraints,
                                    scopes,
                                );
                                if let Some(return_local) = return_local {
                                    mir.push_instr(
                                        block,
                                        Instr::Copy {
                                            target: return_local,
                                            source,
                                        },
                                    );
                                }
                                (ty, block)
                            }
                            None => (InferredTy::Concrete(Ty::UNIT), block),
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
                        mir.set_terminator(block, Terminator::Return);
                    }
                }
                // dandling block for the unreachable subsequent code
                (InferredTy::Concrete(Ty::NEVER), mir.new_block())
            }
            ast::StatementData::Break(stmt) => {
                match scopes.loop_scope() {
                    None => {
                        self.result_builder.add_error(BreakOutsideLoopError {
                            span: statement.span,
                        });
                    }
                    Some(loop_scope) => {
                        if loop_scope.break_ty.is_none() {
                            loop_scope.break_ty = Some(inferrer.fresh());
                        }
                        let break_ty = InferredTy::Variable(loop_scope.break_ty.unwrap());
                        let break_span = loop_scope.span;
                        let target_block = loop_scope.end_block;
                        let (break_expr_ty, block) = match &stmt.value {
                            Some(value) => {
                                if loop_scope.break_var.is_none() {
                                    loop_scope.break_var = Some(mir.new_variable());
                                }
                                let break_var = loop_scope.break_var.unwrap();
                                let (ty, block, source) = self.analyze_expr(
                                    value,
                                    block,
                                    mir,
                                    inferrer,
                                    constraints,
                                    scopes,
                                );
                                let target =
                                    mir.new_local(ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                                mir.push_instr(block, Instr::Copy { target, source });
                                mir.write_variable(break_var, block, target);
                                (ty, block)
                            }
                            None => (InferredTy::Concrete(Ty::UNIT), block),
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
                        mir.set_terminator(block, Terminator::Jump { target_block });
                    }
                }
                // dandling block for the unreachable subsequent code
                (InferredTy::Concrete(Ty::NEVER), mir.new_block())
            }
            ast::StatementData::Continue(_) => {
                match scopes.loop_scope() {
                    None => self.result_builder.add_error(ContinueOutsideLoopError {
                        span: statement.span,
                    }),
                    Some(loop_scope) => mir.set_terminator(
                        block,
                        Terminator::Jump {
                            target_block: loop_scope.start_block,
                        },
                    ),
                }
                // dandling block for the unreachable subsequent code
                (InferredTy::Concrete(Ty::NEVER), mir.new_block())
            }
            ast::StatementData::Yield(stmt) => {
                match scopes.block_scope() {
                    Some(block_scope) => {
                        if block_scope.yield_ty.is_none() {
                            block_scope.yield_ty = Some(inferrer.fresh());
                        }
                        let yield_ty = InferredTy::Variable(block_scope.yield_ty.unwrap());
                        let yield_span = block_scope.span;
                        if block_scope.yield_var.is_none() {
                            block_scope.yield_var = Some(mir.new_variable());
                        }
                        let yield_var = block_scope.yield_var.unwrap();
                        let (yield_expr_ty, block, source) = self.analyze_expr(
                            &stmt.value,
                            block,
                            mir,
                            inferrer,
                            constraints,
                            scopes,
                        );
                        let target =
                            mir.new_local(yield_expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(block, Instr::Copy { target, source });
                        mir.write_variable(yield_var, block, target);
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
                // yield terminator is handled in the consumer code
                (InferredTy::Concrete(Ty::NEVER), block)
            }
        }
    }

    fn analyze_expr_unit(
        &mut self,
        expr: &ast::Expr,
        block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (InferredTy, BlockId) {
        match &expr.data {
            ast::ExprData::Block(ast_block) => {
                scopes.enter_block_scope(expr.span);
                let (block_expr_ty, block) =
                    self.analyze_block(ast_block, block, mir, inferrer, constraints, scopes);
                let (mut expr_ty, block) = match block_expr_ty {
                    Ty::NEVER => {
                        // Either it is an infinite loop, or it yields explicitly
                        // no constraints to add
                        (Ty::NEVER, mir.new_block())
                    }
                    block_expr_ty => {
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: InferredTy::Concrete(block_expr_ty),
                            ty1_span: expr.span,
                            ty2: InferredTy::Concrete(Ty::UNIT),
                            ty2_span: expr.span,
                        });
                        (Ty::UNIT, block)
                    }
                };
                if let Some(yield_ty) = scopes.block_scope().unwrap().yield_ty {
                    constraints.push_back(InferenceConstraint::Equal {
                        ty1: InferredTy::Variable(yield_ty),
                        ty1_span: expr.span,
                        ty2: InferredTy::Concrete(Ty::UNIT),
                        ty2_span: expr.span,
                    });
                    // the above will fail as you cannot yield a unit
                    // return an error type in this case
                    expr_ty = Ty::ERROR;
                }
                scopes.exit_block_scope();
                (InferredTy::Concrete(expr_ty), block)
            }
            ast::ExprData::If(if_expr) => {
                let (cond_ty, block, source) = self.analyze_expr(
                    &if_expr.condition,
                    block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );
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
                let target = mir.new_local(Ty::U1);
                mir.push_instr(block, Instr::Copy { target, source });
                let then_block = mir.new_block();
                let else_block = mir.new_block();
                let end_block = if if_expr.false_block.is_some() {
                    mir.new_block()
                } else {
                    else_block
                };
                mir.set_terminator(
                    block,
                    Terminator::JumpIf {
                        source: target,
                        condition: Condition::NotZero,
                        then_block,
                        else_block,
                    },
                );
                mir.seal_block(then_block);
                mir.seal_block(else_block);
                let (true_block_ty, then_block) = self.analyze_expr_unit(
                    &if_expr.true_block,
                    then_block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );

                mir.set_terminator(
                    then_block,
                    Terminator::Jump {
                        target_block: end_block,
                    },
                );
                let false_block_ty = match if_expr.false_block.as_ref() {
                    None => InferredTy::Concrete(Ty::UNIT),
                    Some(false_block) => {
                        let (false_block_ty, else_block) = self.analyze_expr_unit(
                            false_block,
                            else_block,
                            mir,
                            inferrer,
                            constraints,
                            scopes,
                        );
                        mir.set_terminator(
                            else_block,
                            Terminator::Jump {
                                target_block: end_block,
                            },
                        );
                        false_block_ty
                    }
                };
                mir.seal_block(end_block);
                if true_block_ty != InferredTy::Concrete(Ty::NEVER) {
                    constraints.push_back(InferenceConstraint::Equal {
                        ty1: true_block_ty.clone(),
                        ty1_span: if_expr.true_block.span,
                        ty2: InferredTy::Concrete(Ty::UNIT),
                        ty2_span: expr.span,
                    });
                }
                if false_block_ty != InferredTy::Concrete(Ty::NEVER) {
                    constraints.push_back(InferenceConstraint::Equal {
                        ty1: false_block_ty.clone(),
                        ty1_span: if_expr
                            .false_block
                            .as_deref()
                            .map(|expr| expr.span)
                            .unwrap_or(expr.span),
                        ty2: InferredTy::Concrete(Ty::UNIT),
                        ty2_span: expr.span,
                    });
                }
                let ty = if true_block_ty == InferredTy::Concrete(Ty::NEVER)
                    && false_block_ty == InferredTy::Concrete(Ty::NEVER)
                {
                    InferredTy::Concrete(Ty::NEVER)
                } else {
                    InferredTy::Concrete(Ty::UNIT)
                };
                (ty, end_block)
            }
            ast::ExprData::Loop(loop_expr) => {
                let start_block = mir.new_block();
                let end_block = mir.new_block();
                mir.set_terminator(
                    block,
                    Terminator::Jump {
                        target_block: start_block,
                    },
                );
                scopes.enter_loop_scope(expr.span, start_block, end_block);
                let (ty, block) = self.analyze_block(
                    &loop_expr.body,
                    start_block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );
                if ty != Ty::NEVER {
                    // if the block has type never,
                    // it never completes an iteration successfully,
                    // so no jump is required
                    mir.set_terminator(
                        block,
                        Terminator::Jump {
                            target_block: start_block,
                        },
                    );
                }
                mir.seal_block(start_block);
                mir.seal_block(end_block);
                let loop_scope = scopes.loop_scope().unwrap();
                let expr_ty = match loop_scope.break_ty {
                    None => {
                        // no break, loop is an infinite loop
                        InferredTy::Concrete(Ty::NEVER)
                    }
                    Some(break_ty) => {
                        constraints.push_back(InferenceConstraint::Equal {
                            ty1: InferredTy::Variable(break_ty),
                            ty1_span: expr.span,
                            ty2: InferredTy::Concrete(Ty::UNIT),
                            ty2_span: expr.span,
                        });
                        InferredTy::Concrete(Ty::UNIT)
                    }
                };
                scopes.exit_loop_scope();
                (expr_ty, end_block)
            }
            _ => {
                let (expr_ty, block, _) =
                    self.analyze_expr(expr, block, mir, inferrer, constraints, scopes);
                (expr_ty, block)
            }
        }
    }

    fn analyze_expr(
        &mut self,
        expr: &ast::Expr,
        block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (InferredTy, BlockId, Operand) {
        match &expr.data {
            ast::ExprData::Place(place) => {
                let (ty, block, source) =
                    self.analyze_place_expr(place, block, mir, inferrer, constraints, scopes);
                let target = mir.new_local(ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(block, Instr::Copy { target, source });
                (ty, block, Operand::Local(target))
            }
            ast::ExprData::Integer(literal) => {
                let int_ty = InferredTy::Variable(inferrer.fresh());
                constraints.push_back(InferenceConstraint::IsInt {
                    ty: int_ty.clone(),
                    span: expr.span,
                });
                (int_ty, block, Operand::Literal(literal.clone()))
            }
            ast::ExprData::Unary(op) => {
                let (expr_ty, block, source) =
                    self.analyze_expr(&op.expr, block, mir, inferrer, constraints, scopes);
                let target = match op.kind {
                    UnaryOpKind::Neg => {
                        constraints.push_back(InferenceConstraint::IsSignedInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                        let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(block, Instr::Neg { target, source });
                        target
                    }
                    kind => {
                        constraints.push_back(InferenceConstraint::IsInt {
                            ty: expr_ty.clone(),
                            span: expr.span,
                        });
                        let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(
                            block,
                            match kind {
                                UnaryOpKind::Not => Instr::Not { target, source },
                                UnaryOpKind::ShiftLeft => Instr::ShiftLeft { target, source },
                                UnaryOpKind::ShiftRight => Instr::ShiftRight { target, source },
                                UnaryOpKind::RotateLeft => Instr::RotateLeft { target, source },
                                UnaryOpKind::RotateRight => Instr::RotateRight { target, source },
                                UnaryOpKind::Neg => unimplemented!("neg already handled"),
                            },
                        );
                        target
                    }
                };
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::Logical(op) => {
                // currently this branch incorrectly handles `And` and `Or` as
                // `BitAnd` and `BitOr`. Will follow this up in a subsequent change
                let (left_ty, block, left) =
                    self.analyze_expr(&op.left, block, mir, inferrer, constraints, scopes);
                let (right_ty, block, right) =
                    self.analyze_expr(&op.right, block, mir, inferrer, constraints, scopes);
                let expr_ty = InferredTy::Concrete(Ty::U1);
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
                let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(
                    block,
                    match op.kind {
                        LogicalOpKind::And => Instr::BitAnd {
                            target,
                            left,
                            right,
                        },
                        LogicalOpKind::Or => Instr::BitOr {
                            target,
                            left,
                            right,
                        },
                    },
                );
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::Arithmetic(op) => {
                let (left_ty, block, left) =
                    self.analyze_expr(&op.left, block, mir, inferrer, constraints, scopes);
                let (right_ty, block, right) =
                    self.analyze_expr(&op.right, block, mir, inferrer, constraints, scopes);
                let expr_ty = InferredTy::Variable(inferrer.fresh());
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
                let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(
                    block,
                    match op.kind {
                        ArithmeticOpKind::BitAnd => Instr::BitAnd {
                            target,
                            left,
                            right,
                        },
                        ArithmeticOpKind::BitOr => Instr::BitOr {
                            target,
                            left,
                            right,
                        },
                        ArithmeticOpKind::BitXor => Instr::BitXor {
                            target,
                            left,
                            right,
                        },
                        ArithmeticOpKind::Add => Instr::Add {
                            target,
                            left,
                            right,
                        },
                        ArithmeticOpKind::Sub => Instr::Sub {
                            target,
                            left,
                            right,
                        },
                    },
                );
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::Comparison(op) => {
                let (left_ty, block, left) =
                    self.analyze_expr(&op.left, block, mir, inferrer, constraints, scopes);
                let (right_ty, block, right) =
                    self.analyze_expr(&op.right, block, mir, inferrer, constraints, scopes);
                let expr_ty = InferredTy::Concrete(Ty::U1);
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
                let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(
                    block,
                    match op.kind {
                        ComparisonOpKind::Eq => Instr::Eq {
                            target,
                            left,
                            right,
                        },
                        ComparisonOpKind::Ne => Instr::Ne {
                            target,
                            left,
                            right,
                        },
                        ComparisonOpKind::Lt => Instr::Lt {
                            target,
                            left,
                            right,
                        },
                        ComparisonOpKind::Le => Instr::Le {
                            target,
                            left,
                            right,
                        },
                        ComparisonOpKind::Gt => Instr::Gt {
                            target,
                            left,
                            right,
                        },
                        ComparisonOpKind::Ge => Instr::Ge {
                            target,
                            left,
                            right,
                        },
                    },
                );
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::Assign(op) => {
                let (left_ty, block, left) =
                    self.resolve_place(&op.left, block, mir, inferrer, constraints, scopes);
                let (right_ty, block, right) =
                    self.analyze_expr(&op.right, block, mir, inferrer, constraints, scopes);
                let expr_ty = InferredTy::Variable(inferrer.fresh());
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
                let target = match op.kind {
                    AssignOpKind::Assign => {
                        let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(
                            block,
                            Instr::Copy {
                                target,
                                source: right,
                            },
                        );
                        target
                    }
                    kind => {
                        let left = match &left {
                            None => {
                                // no error needs to be added in this situation as it
                                // should already have been added in resolve_place()
                                let target = mir.new_local(Ty::ERROR);
                                mir.push_instr(block, Instr::Undefined { target });
                                return (expr_ty, block, Operand::Local(target));
                            }
                            Some(ResolvedPlace::Function(_)) => {
                                self.result_builder
                                    .add_error(FnPtrNotAssignableError { span: op.left.span });
                                let target = mir.new_local(Ty::ERROR);
                                mir.push_instr(block, Instr::Undefined { target });
                                return (expr_ty, block, Operand::Local(target));
                            }
                            Some(ResolvedPlace::Variable(variable)) => {
                                let local = mir.read_variable(*variable, block);
                                Operand::Local(local)
                            }
                            Some(ResolvedPlace::Global(source)) => {
                                let target =
                                    mir.new_local(left_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                                mir.push_instr(
                                    block,
                                    Instr::Load {
                                        target,
                                        source: source.clone(),
                                    },
                                );
                                Operand::Local(target)
                            }
                        };

                        let target = mir.new_local(expr_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(
                            block,
                            match kind {
                                AssignOpKind::AddAssign => Instr::Add {
                                    target,
                                    left,
                                    right,
                                },
                                AssignOpKind::SubAssign => Instr::Sub {
                                    target,
                                    left,
                                    right,
                                },
                                AssignOpKind::BitAndAssign => Instr::BitAnd {
                                    target,
                                    left,
                                    right,
                                },
                                AssignOpKind::BitOrAssign => Instr::BitOr {
                                    target,
                                    left,
                                    right,
                                },
                                AssignOpKind::BitXorAssign => Instr::BitXor {
                                    target,
                                    left,
                                    right,
                                },
                                AssignOpKind::Assign => {
                                    unreachable!("assign handled separately")
                                }
                            },
                        );
                        target
                    }
                };

                match left {
                    None => {
                        // no error needs to be added in this situation as it
                        // should already have been added in resolve_place()
                        let target = mir.new_local(Ty::ERROR);
                        mir.push_instr(block, Instr::Undefined { target });
                        return (expr_ty, block, Operand::Local(target));
                    }
                    Some(ResolvedPlace::Function(_)) => {
                        self.result_builder
                            .add_error(FnPtrNotAssignableError { span: op.left.span });
                        let target = mir.new_local(Ty::ERROR);
                        mir.push_instr(block, Instr::Undefined { target });
                        return (expr_ty, block, Operand::Local(target));
                    }
                    Some(ResolvedPlace::Variable(variable)) => {
                        mir.write_variable(variable, block, target);
                    }
                    Some(ResolvedPlace::Global(global)) => {
                        mir.push_instr(
                            block,
                            Instr::Store {
                                target: global,
                                source: Operand::Local(target),
                            },
                        );
                    }
                }
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::Block(ast_block) => {
                scopes.enter_block_scope(expr.span);
                let (block_expr_ty, block) =
                    self.analyze_block(ast_block, block, mir, inferrer, constraints, scopes);
                let expr_ty = match scopes.block_scope().unwrap().yield_ty {
                    None => {
                        let expr_ty = if block_expr_ty == Ty::NEVER {
                            Ty::NEVER
                        } else {
                            self.result_builder
                                .add_error(YieldMissingError { span: expr.span });
                            Ty::ERROR
                        };
                        InferredTy::Concrete(expr_ty)
                    }
                    Some(yield_ty) => InferredTy::Variable(yield_ty),
                };
                let (block, target) = match scopes.block_scope().unwrap().yield_var {
                    Some(yield_var) => (block, mir.read_variable(yield_var, block)),
                    None => {
                        let target = mir.new_local(Ty::ERROR);
                        let block = if expr_ty == InferredTy::Concrete(Ty::NEVER) {
                            // create a dangling block for a `never` as all subsequent code is
                            // unreachable
                            mir.new_block()
                        } else {
                            block
                        };
                        // either it's a dangling block (due to a never) or an error was
                        // encountered above. In either case, it is 'safe' to add an undefined
                        // instruction as it will either be dropped from successful MIR or
                        // will error out before producing the final MIR
                        mir.push_instr(block, Instr::Undefined { target });
                        (block, target)
                    }
                };
                scopes.exit_block_scope();
                (expr_ty, block, Operand::Local(target))
            }
            ast::ExprData::If(if_expr) => {
                let (cond_ty, block, source) = self.analyze_expr(
                    &if_expr.condition,
                    block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );
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
                let target = mir.new_local(Ty::U1);
                mir.push_instr(block, Instr::Copy { target, source });
                let then_block = mir.new_block();
                let else_block = mir.new_block();
                let end_block = mir.new_block();
                let variable = mir.new_variable();
                mir.set_terminator(
                    block,
                    Terminator::JumpIf {
                        source: target,
                        condition: Condition::NotZero,
                        then_block,
                        else_block,
                    },
                );
                mir.seal_block(then_block);
                mir.seal_block(else_block);
                let (true_block_ty, then_block, then_source) = self.analyze_expr(
                    &if_expr.true_block,
                    then_block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );
                let target = mir.new_local(true_block_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(
                    then_block,
                    Instr::Copy {
                        target,
                        source: then_source,
                    },
                );
                mir.write_variable(variable, then_block, target);
                mir.set_terminator(
                    then_block,
                    Terminator::Jump {
                        target_block: end_block,
                    },
                );
                let false_block_ty = match if_expr.false_block.as_ref() {
                    None => {
                        let target = mir.new_local(Ty::UNIT);
                        mir.push_instr(else_block, Instr::Undefined { target });
                        mir.write_variable(variable, else_block, target);
                        InferredTy::Concrete(Ty::UNIT)
                    }
                    Some(false_block) => {
                        let (false_block_ty, else_block, source) = self.analyze_expr(
                            false_block,
                            else_block,
                            mir,
                            inferrer,
                            constraints,
                            scopes,
                        );
                        let target =
                            mir.new_local(false_block_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                        mir.push_instr(else_block, Instr::Copy { target, source });
                        mir.write_variable(variable, else_block, target);
                        false_block_ty
                    }
                };
                mir.set_terminator(
                    else_block,
                    Terminator::Jump {
                        target_block: end_block,
                    },
                );
                mir.seal_block(end_block);
                let ty = match (true_block_ty, false_block_ty) {
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
                };
                let local = mir.read_variable(variable, end_block);
                (ty, end_block, Operand::Local(local))
            }
            ast::ExprData::Loop(loop_expr) => {
                let start_block = mir.new_block();
                let end_block = mir.new_block();
                mir.set_terminator(
                    block,
                    Terminator::Jump {
                        target_block: start_block,
                    },
                );
                scopes.enter_loop_scope(expr.span, start_block, end_block);
                let (ty, block) = self.analyze_block(
                    &loop_expr.body,
                    start_block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );
                if ty != Ty::NEVER {
                    // if the block has type never,
                    // it never completes an iteration successfully,
                    // so no jump is required
                    mir.set_terminator(
                        block,
                        Terminator::Jump {
                            target_block: start_block,
                        },
                    );
                }
                mir.seal_block(start_block);
                mir.seal_block(end_block);
                let loop_scope = scopes.loop_scope().unwrap();
                let expr_ty = match loop_scope.break_ty {
                    None => {
                        // no break, loop is an infinite loop
                        InferredTy::Concrete(Ty::NEVER)
                    }
                    Some(break_ty) => InferredTy::Variable(break_ty),
                };
                let target = match loop_scope.break_var {
                    Some(break_var) => mir.read_variable(break_var, end_block),
                    None => mir.new_local(Ty::NEVER),
                };
                scopes.exit_loop_scope();
                (expr_ty, end_block, Operand::Local(target))
            }
            ast::ExprData::Call(call_expr) => {
                let (place_ty, block, callee) = self.analyze_place_expr(
                    &call_expr.callee,
                    block,
                    mir,
                    inferrer,
                    constraints,
                    scopes,
                );

                let return_ty = inferrer.fresh();

                constraints.push_back(InferenceConstraint::IsFnPtr {
                    ty: place_ty,
                    span: expr.span,
                    return_ty,
                });

                let callee = match callee {
                    Operand::Local(local) => Callee::Local(local),
                    Operand::FnPtr(function) => Callee::FnPtr(function),
                    callee => {
                        let target = mir.new_local(Ty::U16);
                        mir.push_instr(
                            block,
                            Instr::Copy {
                                target,
                                source: callee,
                            },
                        );
                        Callee::Local(target)
                    }
                };
                let return_block = mir.new_block();
                mir.set_terminator(
                    block,
                    Terminator::Call {
                        return_block,
                        callee,
                    },
                );
                mir.seal_block(return_block);
                let target = mir.new_local(Ty::UNRESOLVED);
                mir.push_instr(return_block, Instr::ReturnValue { target });
                (
                    InferredTy::Variable(return_ty),
                    return_block,
                    Operand::Local(target),
                )
            }
            ast::ExprData::Paren(expr) => {
                self.analyze_expr(expr, block, mir, inferrer, constraints, scopes)
            }
        }
    }

    fn analyze_place_expr(
        &mut self,
        place: &ast::Place,
        block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (InferredTy, BlockId, Operand) {
        let (place_ty, block, resolved_place) =
            self.resolve_place(place, block, mir, inferrer, constraints, scopes);
        let resolved_place = match resolved_place {
            Some(resolved_place) => resolved_place,
            None => {
                let target = mir.new_local(Ty::ERROR);
                mir.push_instr(block, Instr::Undefined { target });
                return (place_ty, block, Operand::Local(target));
            }
        };
        match resolved_place {
            ResolvedPlace::Function(fn_ptr) => (place_ty, block, Operand::FnPtr(fn_ptr)),
            ResolvedPlace::Variable(variable) => {
                let local = mir.read_variable(variable, block);
                (place_ty, block, Operand::Local(local))
            }
            ResolvedPlace::Global(source) => {
                let target = mir.new_local(place_ty.to_concrete().unwrap_or(Ty::UNRESOLVED));
                mir.push_instr(block, Instr::Load { target, source });
                (place_ty, block, Operand::Local(target))
            }
        }
    }

    fn resolve_place(
        &mut self,
        place: &ast::Place,
        mut block: BlockId,
        mir: &mut FunctionBuilder,
        inferrer: &mut Inferrer,
        constraints: &mut VecDeque<InferenceConstraint>,
        scopes: &mut Scopes,
    ) -> (InferredTy, BlockId, Option<ResolvedPlace>) {
        let (mut place_ty, mut place_span, place_id) = match scopes.get_place(&place.name.data) {
            Some(place_scope) => (place_scope.ty.clone(), place.name.span, place_scope.mir_id),
            None => {
                self.result_builder
                    .add_error(NotDefinedError::from(place.name.clone()));
                return (InferredTy::Concrete(Ty::ERROR), block, None);
            }
        };
        if place.projections.is_empty() {
            return match place_id {
                PlaceId::Function(function) => {
                    (place_ty, block, Some(ResolvedPlace::Function(function)))
                }
                PlaceId::Variable(variable) => {
                    (place_ty, block, Some(ResolvedPlace::Variable(variable)))
                }
                PlaceId::Global(global) => (
                    place_ty,
                    block,
                    Some(ResolvedPlace::Global(GlobalPlace {
                        global,
                        projections: Vec::new(),
                    })),
                ),
            };
        }
        let mut global_place = match place_id {
            PlaceId::Global(global) => Some(GlobalPlace {
                global,
                projections: Vec::new(),
            }),
            _ => None,
        };
        for part in &place.projections {
            let (new_place_ty, new_place_span, new_block, projection) = match part {
                PlaceProjection::Field(field) => {
                    let field_ty = inferrer.fresh();
                    constraints.push_back(InferenceConstraint::HasField {
                        ty: place_ty,
                        place_span,
                        field: field.clone(),
                        field_ty,
                    });
                    (
                        InferredTy::Variable(field_ty),
                        field.span,
                        block,
                        GlobalPlaceProjection::Field(field.data.clone()),
                    )
                }
                PlaceProjection::Index(index) => {
                    let (index_expr_ty, block, target) =
                        self.analyze_expr(index, block, mir, inferrer, constraints, scopes);
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
                    (
                        InferredTy::Variable(item_ty),
                        index.span,
                        block,
                        GlobalPlaceProjection::Index(target),
                    )
                }
            };
            if let Some(global_place) = global_place.as_mut() {
                global_place.projections.push(projection);
            }
            place_ty = new_place_ty;
            place_span = new_place_span;
            block = new_block;
        }
        (place_ty, block, global_place.map(ResolvedPlace::Global))
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
                                match data.fields.get(&field.data) {
                                    None => {
                                        self.result_builder.add_error(FieldNotDefinedError {
                                            struct_span: data.name.span,
                                            ty: ty.display(&self.ty_registry).into_detached(),
                                            field: field.data,
                                            field_span: field.span,
                                        });
                                        Ty::ERROR
                                    }
                                    Some(field_data) => field_data.ty,
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
    fn insert_place(
        &mut self,
        name: SpannedIdentifier,
        ty: InferredTy,
        mir_id: PlaceId,
    ) -> Option<PlaceScope> {
        if self.global_frozen {
            assert!(self.place_scopes.len() > 1, "global scope is frozen");
        } else {
            assert_eq!(self.place_scopes.len(), 1, "not in global scope");
        }
        self.place_scopes
            .last_mut()
            .expect("not in place scope")
            .insert(name.data.clone(), PlaceScope { name, ty, mir_id })
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
            yield_var: None,
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

    fn enter_loop_scope(&mut self, span: SourceSpan, start_block: BlockId, end_block: BlockId) {
        self.loop_scopes.push(LoopScope {
            span,
            break_ty: None,
            break_var: None,
            start_block,
            end_block,
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

    fn enter_fn_scope(
        &mut self,
        return_ty: InferredTy,
        span: SourceSpan,
        return_local: Option<LocalId>,
    ) {
        assert!(self.fn_scope.is_none(), "already in fn scope");
        assert_eq!(self.place_scopes.len(), 1, "missing global place scope");
        self.fn_scope = Some(FnScope {
            return_ty,
            span,
            return_local,
        });
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
    mir_id: PlaceId,
}

#[derive(Clone, Copy, derive_more::From)]
enum PlaceId {
    Function(FunctionId),
    Variable(Variable),
    Global(GlobalId),
}

enum ResolvedPlace {
    Function(FunctionId),
    Variable(Variable),
    Global(GlobalPlace),
}

struct FnScope {
    return_ty: InferredTy,
    span: SourceSpan,
    return_local: Option<LocalId>,
}

struct BlockScope {
    yield_ty: Option<TyVar>,
    span: SourceSpan,
    yield_var: Option<Variable>,
}

struct LoopScope {
    break_ty: Option<TyVar>,
    break_var: Option<Variable>,
    span: SourceSpan,
    start_block: BlockId,
    end_block: BlockId,
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
