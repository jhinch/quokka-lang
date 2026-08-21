use crate::analyze::error::*;
use crate::analyze::infer::{InferredTy, TyVar};
use crate::analyze::solver::TypeSolver;
use crate::ast::{
    self, ArithmeticOpKind, AssignOpKind, ComparisonOpKind, Identifier, IntegerLiteral,
    LogicalOpKind, SpannedIdentifier, TyRefData, UnaryOpKind,
};
use crate::attr::{
    ConstDeclAttributes, FieldDeclAttributes, FnDeclAttributes, GlobalDeclAttributes,
    StructDeclAttributes,
};
use crate::consteval;
use crate::mir::{
    BlockId, Callee, Condition, FunctionBuilder, FunctionId, GlobalId, Instr, LocalId, Mir,
    MirBuilder, Operand, PlaceProjection, Terminator, Variable,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::num::BitCount;
use crate::source::span::span_between;
use crate::ty::{StructFieldTyData, Ty, TyRegistry};
use indexmap::{IndexMap, IndexSet};
use miette::SourceSpan;
use slotmap::SparseSecondaryMap;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::instrument;

pub mod error;
pub mod infer;
pub mod solver;

#[derive(Default)]
pub struct Analyzer {
    ty_registry: TyRegistry,
    mir_builder: MirBuilder,
    result_builder: MultiResultBuilder,
}

impl Analyzer {
    #[instrument(skip(self, root))]
    pub fn analyze(mut self, root: &ast::Root) -> Result<(Mir, TyRegistry), MultiError> {
        self.register_types(root);
        self.check_errors()?;
        let AnalyzedGlobalScope {
            mut scopes,
            fn_decls,
            const_decls,
        } = self.define_global_scope(root);

        let mut compiled_const_decls = IndexMap::new();
        for (const_decl_id, (const_decl, ty)) in const_decls {
            let (mir, used_const_decls) = self.analyze_const_decl(const_decl, ty, &mut scopes);
            compiled_const_decls.insert(const_decl_id, (mir, used_const_decls));
        }
        for (function_id, (fn_decl, ty, attrs)) in fn_decls {
            self.analyze_fn_decl(fn_decl, ty, attrs, function_id, &mut scopes);
        }
        self.check_errors()?;
        self.initialize_const_decls(compiled_const_decls);
        self.check_errors()?;
        Ok((self.mir_builder.build(), self.ty_registry))
    }

    fn check_errors(&mut self) -> Result<(), MultiError> {
        self.result_builder.take().finish(()).into_result()
    }

    fn register_types(&mut self, root: &ast::Root) {
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
                let mut fields = Vec::with_capacity(decl.fields.len());
                let mut field_name_to_index = HashMap::with_capacity(decl.fields.len());
                let mut failed = false;
                for field in &decl.fields {
                    let ty = self.resolve_ty_ref(&field.ty);
                    let field_attrs = FieldDeclAttributes::try_from(field.attributes.as_slice())
                        .unwrap_or_else(|e| {
                            self.result_builder.add_error(e);
                            FieldDeclAttributes::default()
                        });
                    if field.name.data != Identifier::unnamed() {
                        match field_name_to_index.entry(field.name.data.clone()) {
                            Entry::Vacant(entry) => {
                                entry.insert(fields.len());
                            }
                            Entry::Occupied(entry) => {
                                let defined = decl.fields[*entry.get()].name.span;
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
                    fields.push((&field.name, ty, field.ty.span, field_attrs));
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
                let mut bit_size = BitCount::new(0);
                let mut fields = IndexMap::new();
                for (field_name, field_ty, field_ty_span, field_attrs) in &type_fields[&ty] {
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
                    let field_bit_size = field_ty.bit_size(&self.ty_registry).padded();
                    if field_name.data == Identifier::unnamed() {
                        if *field_ty != Ty::ERROR && !field_ty.is_unsigned_int() {
                            let mut expected = Vec::new();
                            let mut field_bit_size = field_bit_size.into_u32();
                            while field_bit_size > 0 {
                                let expected_field_size = match field_bit_size {
                                    1 => 1,
                                    2..=3 => 2,
                                    4..=7 => 4,
                                    _ => 8,
                                };
                                expected.push(format!("_: u{expected_field_size}"));
                                field_bit_size -= expected_field_size;
                            }
                            self.result_builder
                                .add_error(UnnamedFieldNotUnsignedIntError {
                                    span: span_between(field_name.span, *field_ty_span),
                                    ty: field_ty.display(&self.ty_registry).into_detached(),
                                    expected: if expected.is_empty() {
                                        None
                                    } else {
                                        Some(format!("expected `{}`", expected.join(", ")))
                                    },
                                });
                        }
                    } else {
                        fields.insert(
                            field_name.data.clone(),
                            StructFieldTyData {
                                name: (*field_name).clone(),
                                ty: *field_ty,
                                ty_span: *field_ty_span,
                                bit_offset: bit_size,
                                attrs: field_attrs.clone(),
                            },
                        );
                    }
                    bit_size += field_bit_size;
                }
                self.ty_registry
                    .complete_struct_registration(ty, fields, bit_size);
                unprocessed_types.shift_remove(&ty);
            }
        }
    }

    fn define_global_scope<'ast>(&mut self, root: &'ast ast::Root) -> AnalyzedGlobalScope<'ast> {
        let mut scopes = Scopes::default();
        scopes.enter_global_scope();
        let mut fn_decls = IndexMap::new();
        let mut const_decls = IndexMap::new();
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
                    fn_decls.insert(function_id, (decl, fn_ptr_ty, attrs));
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
                    let global_id = self.mir_builder.insert_global(
                        decl.name.clone(),
                        const_ty,
                        attrs.clone().into(),
                    );
                    const_decls.insert(global_id, (decl, const_ty));
                    (
                        &decl.name,
                        InferredTy::Concrete(const_ty),
                        PlaceId::from(global_id),
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
        AnalyzedGlobalScope {
            scopes,
            fn_decls,
            const_decls,
        }
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
    ) -> (FunctionBuilder, SparseSecondaryMap<GlobalId, SourceSpan>) {
        let mut mir = FunctionBuilder::new(const_decl.name.data.clone(), const_ty);
        let target = mir.create_return_local(const_ty);

        scopes.enter_const_decl_scope();

        let mut type_solver = TypeSolver::default();

        let block = mir.new_block();
        mir.seal_block(block);

        let (block, inferred_ty, source) =
            self.analyze_expr(&const_decl.value, block, &mut mir, &mut type_solver, scopes);
        type_solver.check_const_decl(const_decl, inferred_ty, const_ty);
        mir.push_instr(
            block,
            Instr::Copy {
                target,
                source: Operand::Local(source),
            },
        );
        mir.set_terminator(block, Terminator::Return);

        let used_const_decls = std::mem::take(
            &mut scopes
                .const_decl_scope()
                .expect("missing const decl scope")
                .used_const_decls,
        );

        scopes.exit_const_decl_scope();

        match type_solver.solve(&mut self.ty_registry) {
            Ok(local_types) => {
                for (local, ty) in local_types {
                    mir.set_local_ty(local, ty);
                }
            }
            Err(errors) => self.result_builder.add_errors(errors),
        }
        mir.propagate_variable_types();
        (mir, used_const_decls)
    }

    fn initialize_const_decls(
        &mut self,
        mut compiled_const_decls: IndexMap<
            GlobalId,
            (FunctionBuilder, SparseSecondaryMap<GlobalId, SourceSpan>),
        >,
    ) {
        compiled_const_decls.reverse();
        let mut completed = HashSet::new();
        let dependents = compiled_const_decls
            .iter()
            .flat_map(|(const_decl_id, (_, used_const_decls))| {
                used_const_decls
                    .keys()
                    .map(|used_const_decl_id| (used_const_decl_id, *const_decl_id))
            })
            .fold(
                HashMap::<GlobalId, HashSet<GlobalId>>::new(),
                |mut map, (used_const_decl_id, const_decl_id)| {
                    map.entry(used_const_decl_id)
                        .or_default()
                        .insert(const_decl_id);
                    map
                },
            );
        while !compiled_const_decls.is_empty() {
            let const_decl_id =
                compiled_const_decls
                    .iter()
                    .rev()
                    .find_map(|(const_decl_id, (_, uses))| {
                        if uses.is_empty() {
                            Some(*const_decl_id)
                        } else {
                            None
                        }
                    });
            let const_decl_id = match const_decl_id {
                Some(const_decl_id) => const_decl_id,
                None => break,
            };
            let (mir, _) = compiled_const_decls
                .swap_remove(&const_decl_id)
                .expect("key just found");
            let mir = mir.build();
            self.mir_builder.set_global_value(
                const_decl_id,
                consteval::evaluate(
                    &mir,
                    &completed
                        .iter()
                        .copied()
                        .map(|global_id| {
                            (
                                global_id,
                                (
                                    self.mir_builder.global_ty(global_id),
                                    self.mir_builder.global_value(global_id),
                                ),
                            )
                        })
                        .collect::<SparseSecondaryMap<_, _>>(),
                    &self.ty_registry,
                ),
            );
            for dependent in dependents.get(&const_decl_id).unwrap_or(&HashSet::new()) {
                compiled_const_decls[dependent].1.remove(const_decl_id);
            }
            completed.insert(const_decl_id);
        }
        for (const_decl_id, (_, uses)) in compiled_const_decls.into_iter().rev() {
            let dependencies = uses.into_iter().map(|(_, v)| v).collect::<Vec<_>>();
            let name = self.mir_builder.global_name(const_decl_id).clone();
            self.result_builder.add_error(CyclicConstDeclError {
                name: name.data,
                span: name.span,
                dependencies,
            });
        }
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
            return_ty,
            fn_decl
                .signature
                .ty
                .as_ref()
                .map(|ty_ref| ty_ref.span)
                .unwrap_or(fn_decl.signature.name.span),
            return_local,
        );

        let mut type_solver = TypeSolver::default();

        let block = mir.new_block();
        mir.seal_block(block);

        let (block, block_ty) =
            self.analyze_block(&fn_decl.body, block, &mut mir, &mut type_solver, scopes);
        match block_ty {
            StatementTy::Never => {
                // Either it is an infinite loop, or it returns explicitly.
                // In either case, there is no constraint to add as the situation
                // an explicit return occurs, the constraint will be handled later on
            }
            StatementTy::Unit => {
                // The block fell off the end. This means it _must_ be a unit even
                // if other branches returned correctly. If the return type is not unit,
                // this indicates that there is a missing return statement. If it is unit,
                // then this is completely legitimate, and we implicitly add the return statement
                if return_ty != Ty::UNIT {
                    let end_offset = fn_decl.body.span.offset() + fn_decl.body.span.len();
                    let start_offset = end_offset - 1;
                    self.result_builder.add_error(ReturnMissingError {
                        span: SourceSpan::from(start_offset..end_offset),
                    });
                }
                mir.set_terminator(block, Terminator::Return);
            }
        }
        scopes.exit_fn_scope();
        match type_solver.solve(&mut self.ty_registry) {
            Ok(local_types) => {
                for (local, ty) in local_types {
                    mir.set_local_ty(local, ty);
                }
            }
            Err(errors) => self.result_builder.add_errors(errors),
        }
        mir.propagate_variable_types();

        self.mir_builder.seal_function(function_id, mir);
    }

    fn analyze_block(
        &mut self,
        ast_block: &ast::Block,
        mut block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, StatementTy) {
        let mut never_found = false;
        let mut unreachable_code: Option<SourceSpan> = None;
        for statement in &ast_block.statements {
            if never_found {
                unreachable_code = Some(match unreachable_code {
                    Some(start_span) => span_between(start_span, statement.span),
                    None => statement.span,
                });
            }
            let (next_block, statement_ty) =
                self.analyze_statement(statement, block, mir, type_solver, scopes);
            block = next_block;
            if statement_ty == StatementTy::Never {
                never_found = true;
            }
        }
        if let Some(span) = unreachable_code {
            self.result_builder.add_error(UnreachableError { span });
        }
        let kind = if never_found {
            StatementTy::Never
        } else {
            StatementTy::Unit
        };
        (block, kind)
    }

    fn analyze_statement(
        &mut self,
        statement: &ast::Statement,
        block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, StatementTy) {
        match &statement.data {
            ast::StatementData::Expr(stmt) => {
                let (block, ty) =
                    self.analyze_expr_unit(&stmt.value, block, mir, type_solver, scopes);
                (block, ty)
            }
            ast::StatementData::Let(stmt) => {
                let let_ty = stmt.ty.as_ref().map(|ty_ref| self.resolve_ty_ref(ty_ref));
                let (block, expr_ty, source) =
                    self.analyze_expr(&stmt.value, block, mir, type_solver, scopes);
                let let_ty = type_solver.check_let_stmt(
                    stmt,
                    statement.span,
                    let_ty,
                    expr_ty,
                    scopes.const_decl_scope().is_some(),
                );
                let target = if mir.is_named_local(source) {
                    // this can occur in simple assignments like:
                    //      let a: u8 = 1;
                    //      let b: u8 = a;
                    // in these situations, add a copy instruction to allow
                    // the new target to be named as the variable.
                    let target = type_solver.new_registered_local(mir, &let_ty);
                    mir.push_instr(
                        block,
                        Instr::Copy {
                            target,
                            source: Operand::Local(source),
                        },
                    );
                    target
                } else {
                    source
                };
                let variable = mir.new_variable();
                mir.set_variable_name(variable, stmt.name.clone());
                mir.write_variable(variable, block, target);
                // ignored the result to allow for local variable shadowing
                let _ = scopes.insert_place(stmt.name.clone(), let_ty, PlaceId::from(variable));
                (block, StatementTy::Unit)
            }
            ast::StatementData::Return(stmt) => {
                match scopes.fn_scope() {
                    None => self.result_builder.add_error(ReturnOutsideFnError {
                        span: statement.span,
                    }),
                    Some(fn_scope) => {
                        let return_ty = fn_scope.return_ty;
                        let return_span = fn_scope.span;
                        let (block, return_expr_ty) = match &stmt.value {
                            Some(value) => {
                                let return_local = fn_scope.return_local;
                                let (block, ty, source) =
                                    self.analyze_expr(value, block, mir, type_solver, scopes);
                                if let Some(return_local) = return_local {
                                    mir.push_instr(
                                        block,
                                        Instr::Copy {
                                            target: return_local,
                                            source: Operand::Local(source),
                                        },
                                    );
                                }
                                (block, Some(ty))
                            }
                            None => (block, None),
                        };
                        type_solver.check_return_stmt(
                            stmt,
                            return_span,
                            statement.span,
                            return_ty,
                            return_expr_ty,
                        );
                        mir.set_terminator(block, Terminator::Return);
                    }
                }
                // dandling block for the unreachable subsequent code
                (mir.new_block(), StatementTy::Never)
            }
            ast::StatementData::Break(stmt) => {
                match scopes.loop_scope() {
                    None => {
                        self.result_builder.add_error(BreakOutsideLoopError {
                            span: statement.span,
                        });
                    }
                    Some(loop_scope) => {
                        loop_scope.break_found = true;
                        let break_ty = loop_scope
                            .break_var
                            .map(|(ty_var, _)| InferredTy::Variable(ty_var));
                        let loop_span = loop_scope.span;
                        let target_block = loop_scope.end_block;
                        let (block, break_expr_ty) = match &stmt.value {
                            Some(value) => {
                                let break_var = loop_scope.break_var.map(|(_, var)| var);
                                let (block, ty, source) =
                                    self.analyze_expr(value, block, mir, type_solver, scopes);
                                if let Some(break_var) = break_var {
                                    mir.write_variable(break_var, block, source);
                                }
                                (block, Some(ty))
                            }
                            None => (block, None),
                        };
                        type_solver.check_break_stmt(
                            stmt,
                            loop_span,
                            statement.span,
                            break_ty,
                            break_expr_ty,
                        );
                        mir.set_terminator(block, Terminator::Jump { target_block });
                    }
                }
                // dandling block for the unreachable subsequent code
                (mir.new_block(), StatementTy::Never)
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
                (mir.new_block(), StatementTy::Never)
            }
            ast::StatementData::Yield(stmt) => {
                let block = match scopes.block_scope() {
                    Some(block_scope) => {
                        let block_span = block_scope.span;
                        let yield_ty = block_scope.yield_ty.map(InferredTy::Variable);
                        let yield_span = block_scope.span;
                        let yield_var = match block_scope.yield_var {
                            Some(yield_var) => yield_var,
                            None => {
                                let yield_var = mir.new_variable();
                                block_scope.yield_var = Some(yield_var);
                                yield_var
                            }
                        };
                        let (block, yield_expr_ty, source) =
                            self.analyze_expr(&stmt.value, block, mir, type_solver, scopes);
                        mir.write_variable(yield_var, block, source);

                        type_solver.check_yield_stmt(
                            stmt,
                            block_span,
                            yield_span,
                            yield_ty,
                            yield_expr_ty,
                        );
                        block
                    }
                    None => {
                        self.result_builder.add_error(YieldOutsideBlockError {
                            span: statement.span,
                        });
                        block
                    }
                };
                // yield terminator is handled in the consumer code
                (block, StatementTy::Never)
            }
        }
    }

    fn analyze_expr_unit(
        &mut self,
        expr: &ast::Expr,
        block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, StatementTy) {
        match &expr.data {
            ast::ExprData::Block(ast_block) => {
                scopes.enter_block_scope(expr.span, None);
                let (block, ty) = self.analyze_block(ast_block, block, mir, type_solver, scopes);
                let block = match ty {
                    StatementTy::Never => {
                        // Either it is an infinite loop, or it yields explicitly.
                        // We both propagate the never type but also create a new block
                        // as the existing block will be pointing to unreachable code
                        mir.new_block()
                    }
                    StatementTy::Unit => {
                        // the block fell off the end. In this situation the block is required to be
                        // unit. In addition, we basic block won't be complete so we need to propagate
                        // it up
                        block
                    }
                };
                scopes.exit_block_scope();
                (block, ty)
            }
            ast::ExprData::If(if_expr) => {
                if scopes.const_decl_scope().is_some() {
                    self.result_builder
                        .add_error(IfNotSupportInConstDeclError { span: expr.span });
                }
                let (block, cond_ty, source) =
                    self.analyze_expr(&if_expr.condition, block, mir, type_solver, scopes);
                type_solver.check_if_expr_condition(if_expr, cond_ty);
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
                        source,
                        condition: Condition::NotZero,
                        then_block,
                        else_block,
                    },
                );
                mir.seal_block(then_block);
                mir.seal_block(else_block);
                let (then_block, true_block_ty) = self.analyze_expr_unit(
                    &if_expr.true_block,
                    then_block,
                    mir,
                    type_solver,
                    scopes,
                );

                self.jump_if_not_divergent(then_block, true_block_ty, end_block, mir);
                let false_block_ty = match if_expr.false_block.as_ref() {
                    None => StatementTy::Unit,
                    Some(false_block) => {
                        let (else_block, false_block_ty) = self.analyze_expr_unit(
                            false_block,
                            else_block,
                            mir,
                            type_solver,
                            scopes,
                        );
                        self.jump_if_not_divergent(else_block, false_block_ty, end_block, mir);
                        false_block_ty
                    }
                };
                mir.seal_block(end_block);
                let outcome = match (true_block_ty, false_block_ty) {
                    (StatementTy::Never, StatementTy::Never) => StatementTy::Never,
                    _ => StatementTy::Unit,
                };
                (end_block, outcome)
            }
            ast::ExprData::Loop(loop_expr) => {
                if scopes.const_decl_scope().is_some() {
                    self.result_builder
                        .add_error(LoopNotSupportInConstDeclError { span: expr.span });
                }
                let start_block = mir.new_block();
                let end_block = mir.new_block();
                mir.set_terminator(
                    block,
                    Terminator::Jump {
                        target_block: start_block,
                    },
                );
                scopes.enter_loop_scope(expr.span, None, start_block, end_block);
                let (block, block_ty) =
                    self.analyze_block(&loop_expr.body, start_block, mir, type_solver, scopes);
                self.jump_if_not_divergent(block, block_ty, start_block, mir);
                mir.seal_block(start_block);
                mir.seal_block(end_block);
                let loop_scope = scopes.loop_scope().unwrap();
                let outcome = if loop_scope.break_found {
                    StatementTy::Unit
                } else {
                    StatementTy::Never
                };
                scopes.exit_loop_scope();
                (end_block, outcome)
            }
            ast::ExprData::Call(call_expr) => {
                let (return_block, _) =
                    self.analyze_call(expr, block, mir, type_solver, scopes, call_expr);
                (return_block, StatementTy::Unit)
            }
            _ => {
                let (block, expr_ty, _) = self.analyze_expr(expr, block, mir, type_solver, scopes);
                let outcome = if expr_ty == InferredTy::Concrete(Ty::NEVER) {
                    StatementTy::Never
                } else {
                    StatementTy::Unit
                };
                (block, outcome)
            }
        }
    }

    fn jump_if_not_divergent(
        &self,
        block: BlockId,
        block_ty: StatementTy,
        target_block: BlockId,
        mir: &mut FunctionBuilder,
    ) {
        match block_ty {
            StatementTy::Never => {
                // if the block has a `never` type it means that a previous
                // basic block already has diverged and the current basic block
                // we are on is dangling dead code. Don't add a terminator to this
                // dangling basic block or it will mess with phi resolution
            }
            StatementTy::Unit => {
                mir.set_terminator(block, Terminator::Jump { target_block });
            }
        }
    }

    fn analyze_expr(
        &mut self,
        expr: &ast::Expr,
        block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, InferredTy, LocalId) {
        match &expr.data {
            ast::ExprData::Place(place) => {
                let (block, ty, source) =
                    self.analyze_place_expr(place, block, mir, type_solver, scopes);
                let target = if let Operand::Local(target) = source {
                    target
                } else {
                    let target = type_solver.new_registered_local(mir, &ty);
                    mir.push_instr(block, Instr::Copy { target, source });
                    target
                };
                (block, ty, target)
            }
            ast::ExprData::Integer(literal) => {
                let ty = type_solver.check_integer_expr(literal, expr.span);
                let target = type_solver.new_registered_local(mir, &ty);
                mir.push_instr(
                    block,
                    Instr::Copy {
                        target,
                        source: Operand::Literal(literal.clone()),
                    },
                );
                (block, ty, target)
            }
            ast::ExprData::Struct(struct_expr) => {
                let ty = self
                    .ty_registry
                    .resolve(&struct_expr.name.data)
                    .unwrap_or_else(|| {
                        self.result_builder
                            .add_error(TypeNotFoundError::from(struct_expr.name.clone()));
                        Ty::ERROR
                    });
                let mut target = mir.new_local(ty);
                mir.push_instr(block, Instr::Alloc { target });
                let mut field_types = Vec::new();
                let mut block = block;
                for field in &struct_expr.fields {
                    let (new_block, field_expr_ty, source) =
                        self.analyze_expr(&field.value, block, mir, type_solver, scopes);
                    field_types.push((&field.name, field_expr_ty, field.value.span));
                    block = new_block;
                    let new_target = mir.new_local(ty);
                    mir.push_instr(
                        block,
                        Instr::Write {
                            target: new_target,
                            left: target,
                            projections: vec![PlaceProjection::Field(field.name.data.clone())],
                            right: Operand::Local(source),
                        },
                    );
                    target = new_target;
                }
                type_solver.check_struct_expr_fields(
                    struct_expr,
                    expr.span,
                    field_types,
                    ty,
                    &self.ty_registry,
                );
                (block, InferredTy::Concrete(ty), target)
            }
            ast::ExprData::Array(array_expr) => {
                let mut block = block;
                let (expr_ty, item_ty) = type_solver.check_array_expr(array_expr, expr.span);
                let mut target = type_solver.new_registered_local(mir, &expr_ty);
                mir.push_instr(block, Instr::Alloc { target });
                for (index, item_expr) in array_expr.items.iter().enumerate() {
                    let (new_block, item_expr_ty, source) =
                        self.analyze_expr(item_expr, block, mir, type_solver, scopes);
                    type_solver.check_item_expr(
                        expr.span,
                        item_expr,
                        item_ty.clone(),
                        item_expr_ty,
                    );
                    block = new_block;
                    let new_target = type_solver.new_registered_local(mir, &expr_ty);
                    mir.push_instr(
                        block,
                        Instr::Write {
                            target: new_target,
                            left: target,
                            projections: vec![PlaceProjection::Index(Operand::Literal(
                                #[allow(clippy::cast_possible_truncation)]
                                IntegerLiteral::from(index as u32),
                            ))],
                            right: Operand::Local(source),
                        },
                    );
                    target = new_target;
                }
                (block, expr_ty, target)
            }
            ast::ExprData::Repeat(repeat_expr) => {
                let (expr_ty, item_ty) = type_solver.check_repeat_expr(repeat_expr, expr.span);
                let mut target = type_solver.new_registered_local(mir, &expr_ty);
                mir.push_instr(block, Instr::Alloc { target });
                let (block, item_expr_ty, source) =
                    self.analyze_expr(&repeat_expr.item, block, mir, type_solver, scopes);
                type_solver.check_item_expr(expr.span, &repeat_expr.item, item_ty, item_expr_ty);
                for index in 0..repeat_expr.count.get() {
                    let new_target = type_solver.new_registered_local(mir, &expr_ty);
                    mir.push_instr(
                        block,
                        Instr::Write {
                            target: new_target,
                            left: target,
                            projections: vec![PlaceProjection::Index(Operand::Literal(
                                IntegerLiteral::from(index as u32),
                            ))],
                            right: Operand::Local(source),
                        },
                    );
                    target = new_target;
                }
                (block, expr_ty, target)
            }
            ast::ExprData::Unary(op) => {
                let (block, inner_ty, source) =
                    self.analyze_expr(&op.expr, block, mir, type_solver, scopes);
                let expr_ty = type_solver.check_unary_expr(op, expr.span, inner_ty);
                let target = type_solver.new_registered_local(mir, &expr_ty);
                mir.push_instr(
                    block,
                    match op.kind {
                        UnaryOpKind::Neg => Instr::Neg { target, source },
                        UnaryOpKind::Not => Instr::Not { target, source },
                        UnaryOpKind::ShiftLeft => Instr::ShiftLeft { target, source },
                        UnaryOpKind::ShiftRight => Instr::ShiftRight { target, source },
                        UnaryOpKind::RotateLeft => Instr::RotateLeft { target, source },
                        UnaryOpKind::RotateRight => Instr::RotateRight { target, source },
                    },
                );
                (block, expr_ty, target)
            }
            ast::ExprData::Logical(op) => {
                let variable = mir.new_variable();
                let (block, left_ty, left) =
                    self.analyze_expr(&op.left, block, mir, type_solver, scopes);
                mir.write_variable(variable, block, left);

                let right_block = mir.new_block();
                let end_block = mir.new_block();

                mir.set_terminator(
                    block,
                    Terminator::JumpIf {
                        source: left,
                        condition: match op.kind {
                            LogicalOpKind::And => Condition::NotZero,
                            LogicalOpKind::Or => Condition::Zero,
                        },
                        then_block: right_block,
                        else_block: end_block,
                    },
                );

                let (right_block, right_ty, right) =
                    self.analyze_expr(&op.right, right_block, mir, type_solver, scopes);
                mir.write_variable(variable, right_block, right);
                mir.set_terminator(
                    right_block,
                    Terminator::Jump {
                        target_block: end_block,
                    },
                );

                mir.seal_block(right_block);
                mir.seal_block(end_block);

                let target = mir.read_variable(variable, end_block);

                let ty = type_solver.check_logical_expr(op, expr.span, left_ty, right_ty);
                (end_block, ty, target)
            }
            ast::ExprData::Arithmetic(op) => {
                let (block, left_ty, left) =
                    self.analyze_expr(&op.left, block, mir, type_solver, scopes);
                let (block, right_ty, right) =
                    self.analyze_expr(&op.right, block, mir, type_solver, scopes);
                let expr_ty = type_solver.check_arithmetic_expr(op, expr.span, left_ty, right_ty);
                let target = type_solver.new_registered_local(mir, &expr_ty);
                mir.push_instr(
                    block,
                    match op.kind {
                        ArithmeticOpKind::BitAnd => Instr::BitAnd {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ArithmeticOpKind::BitOr => Instr::BitOr {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ArithmeticOpKind::BitXor => Instr::BitXor {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ArithmeticOpKind::Add => Instr::Add {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ArithmeticOpKind::Sub => Instr::Sub {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                    },
                );
                (block, expr_ty, target)
            }
            ast::ExprData::Comparison(op) => {
                let (block, left_ty, left) =
                    self.analyze_expr(&op.left, block, mir, type_solver, scopes);
                let (block, right_ty, right) =
                    self.analyze_expr(&op.right, block, mir, type_solver, scopes);
                let expr_ty = type_solver.check_comparison_expr(op, expr.span, left_ty, right_ty);
                let target = type_solver.new_registered_local(mir, &expr_ty);
                mir.push_instr(
                    block,
                    match op.kind {
                        ComparisonOpKind::Eq => Instr::Eq {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ComparisonOpKind::Ne => Instr::Ne {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ComparisonOpKind::Lt => Instr::Lt {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ComparisonOpKind::Le => Instr::Le {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ComparisonOpKind::Gt => Instr::Gt {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                        ComparisonOpKind::Ge => Instr::Ge {
                            target,
                            left,
                            right: Operand::Local(right),
                        },
                    },
                );
                (block, expr_ty, target)
            }
            ast::ExprData::Assign(op) => {
                let (block, left_ty, left) =
                    self.resolve_place(&op.left, block, mir, type_solver, scopes);
                let (block, right_ty, right) =
                    self.analyze_expr(&op.right, block, mir, type_solver, scopes);
                let expr_ty =
                    type_solver.check_assign_expr(op, expr.span, left_ty.clone(), right_ty);
                if let Some(ResolvedPlace::Global(global, _)) = &left
                    && !self.mir_builder.is_mutable_global(*global)
                {
                    self.result_builder
                        .add_error(ConstNotAssignableError { span: op.left.span });
                }
                let target = match op.kind {
                    AssignOpKind::Assign => right,
                    kind => {
                        let left = match &left {
                            None => {
                                // no error needs to be added in this situation as it
                                // should already have been added in resolve_place()
                                let target = mir.new_local(Ty::ERROR);
                                mir.push_instr(block, Instr::Undefined { target });
                                return (block, expr_ty, target);
                            }
                            Some(ResolvedPlace::Function(_)) => {
                                self.result_builder
                                    .add_error(FnPtrNotAssignableError { span: op.left.span });
                                let target = mir.new_local(Ty::ERROR);
                                mir.push_instr(block, Instr::Undefined { target });
                                return (block, expr_ty, target);
                            }
                            Some(ResolvedPlace::Variable(variable, _, projections)) => {
                                let source = mir.read_variable(*variable, block);
                                if projections.is_empty() {
                                    source
                                } else {
                                    let target = type_solver.new_registered_local(mir, &left_ty);
                                    mir.push_instr(
                                        block,
                                        Instr::Read {
                                            target,
                                            source,
                                            projections: projections.clone(),
                                        },
                                    );
                                    target
                                }
                            }
                            Some(ResolvedPlace::Global(source, projections)) => {
                                let target = type_solver.new_registered_local(mir, &left_ty);
                                mir.push_instr(
                                    block,
                                    Instr::Load {
                                        target,
                                        source: *source,
                                        projections: projections.clone(),
                                    },
                                );
                                target
                            }
                        };

                        let target = type_solver.new_registered_local(mir, &expr_ty);
                        mir.push_instr(
                            block,
                            match kind {
                                AssignOpKind::AddAssign => Instr::Add {
                                    target,
                                    left,
                                    right: Operand::Local(right),
                                },
                                AssignOpKind::SubAssign => Instr::Sub {
                                    target,
                                    left,
                                    right: Operand::Local(right),
                                },
                                AssignOpKind::BitAndAssign => Instr::BitAnd {
                                    target,
                                    left,
                                    right: Operand::Local(right),
                                },
                                AssignOpKind::BitOrAssign => Instr::BitOr {
                                    target,
                                    left,
                                    right: Operand::Local(right),
                                },
                                AssignOpKind::BitXorAssign => Instr::BitXor {
                                    target,
                                    left,
                                    right: Operand::Local(right),
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
                        (block, expr_ty, target)
                    }
                    Some(ResolvedPlace::Function(_)) => {
                        self.result_builder
                            .add_error(FnPtrNotAssignableError { span: op.left.span });
                        let target = mir.new_local(Ty::ERROR);
                        mir.push_instr(block, Instr::Undefined { target });
                        (block, expr_ty, target)
                    }
                    Some(ResolvedPlace::Variable(variable, place_ty, projections)) => {
                        let variable_target = if projections.is_empty() {
                            target
                        } else {
                            let write_target = type_solver.new_registered_local(mir, &place_ty);
                            let left = mir.read_variable(variable, block);
                            mir.push_instr(
                                block,
                                Instr::Write {
                                    target: write_target,
                                    left,
                                    projections,
                                    right: Operand::Local(target),
                                },
                            );
                            write_target
                        };
                        mir.write_variable(variable, block, variable_target);
                        (block, expr_ty, target)
                    }
                    Some(ResolvedPlace::Global(global, projections)) => {
                        mir.push_instr(
                            block,
                            Instr::Store {
                                target: global,
                                projections,
                                source: Operand::Local(target),
                            },
                        );
                        (block, expr_ty, target)
                    }
                }
            }
            ast::ExprData::Block(ast_block) => {
                let yield_ty = type_solver.fresh_ty_var();
                scopes.enter_block_scope(expr.span, Some(yield_ty));
                let (block, block_kind) =
                    self.analyze_block(ast_block, block, mir, type_solver, scopes);
                let expr_ty = if scopes.block_scope().unwrap().yield_var.is_some() {
                    InferredTy::Variable(yield_ty)
                } else {
                    let expr_ty = match block_kind {
                        StatementTy::Never => Ty::NEVER,
                        StatementTy::Unit => {
                            let end_offset = expr.span.offset() + expr.span.len();
                            let start_offset = end_offset - 1;
                            self.result_builder.add_error(YieldMissingError {
                                span: SourceSpan::from(start_offset..end_offset),
                            });
                            Ty::ERROR
                        }
                    };
                    InferredTy::Concrete(expr_ty)
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
                (block, expr_ty, target)
            }
            ast::ExprData::If(if_expr) => {
                if scopes.const_decl_scope().is_some() {
                    self.result_builder
                        .add_error(IfNotSupportInConstDeclError { span: expr.span });
                    let target = mir.new_local(Ty::ERROR);
                    mir.push_instr(block, Instr::Undefined { target });
                    return (block, InferredTy::Concrete(Ty::ERROR), target);
                }
                let (block, cond_ty, source) =
                    self.analyze_expr(&if_expr.condition, block, mir, type_solver, scopes);
                type_solver.check_if_expr_condition(if_expr, cond_ty);
                let then_block = mir.new_block();
                let else_block = mir.new_block();
                let end_block = mir.new_block();
                let variable = mir.new_variable();
                mir.set_terminator(
                    block,
                    Terminator::JumpIf {
                        source,
                        condition: Condition::NotZero,
                        then_block,
                        else_block,
                    },
                );
                mir.seal_block(then_block);
                mir.seal_block(else_block);
                let (then_block, true_block_ty, then_source) =
                    self.analyze_expr(&if_expr.true_block, then_block, mir, type_solver, scopes);
                if true_block_ty != InferredTy::Concrete(Ty::NEVER) {
                    mir.write_variable(variable, then_block, then_source);
                }
                self.jump_if_not_divergent(
                    then_block,
                    StatementTy::from(&true_block_ty),
                    end_block,
                    mir,
                );
                let false_block_ty = match if_expr.false_block.as_ref() {
                    None => {
                        let target = mir.new_local(Ty::ERROR);
                        mir.push_instr(else_block, Instr::Undefined { target });
                        mir.write_variable(variable, else_block, target);
                        None
                    }
                    Some(false_block) => {
                        let (else_block, false_block_ty, source) =
                            self.analyze_expr(false_block, else_block, mir, type_solver, scopes);
                        if false_block_ty != InferredTy::Concrete(Ty::NEVER) {
                            mir.write_variable(variable, else_block, source);
                        }
                        self.jump_if_not_divergent(
                            else_block,
                            StatementTy::from(&false_block_ty),
                            end_block,
                            mir,
                        );
                        Some(false_block_ty)
                    }
                };
                mir.seal_block(end_block);
                let ty =
                    type_solver.check_if_expr(if_expr, expr.span, true_block_ty, false_block_ty);
                let local = mir.read_variable(variable, end_block);
                (end_block, ty, local)
            }
            ast::ExprData::Loop(loop_expr) => {
                if scopes.const_decl_scope().is_some() {
                    self.result_builder
                        .add_error(LoopNotSupportInConstDeclError { span: expr.span });
                    let target = mir.new_local(Ty::ERROR);
                    mir.push_instr(block, Instr::Undefined { target });
                    return (block, InferredTy::Concrete(Ty::ERROR), target);
                }
                let start_block = mir.new_block();
                let end_block = mir.new_block();
                mir.set_terminator(
                    block,
                    Terminator::Jump {
                        target_block: start_block,
                    },
                );
                let break_ty = type_solver.fresh_ty_var();
                let break_variable = mir.new_variable();
                scopes.enter_loop_scope(
                    expr.span,
                    Some((break_ty, break_variable)),
                    start_block,
                    end_block,
                );
                let (block, block_ty) =
                    self.analyze_block(&loop_expr.body, start_block, mir, type_solver, scopes);
                self.jump_if_not_divergent(block, block_ty, start_block, mir);
                mir.seal_block(start_block);
                mir.seal_block(end_block);
                let loop_scope = scopes.loop_scope().unwrap();
                let (expr_ty, target) = if loop_scope.break_found {
                    let target = mir.read_variable(break_variable, end_block);
                    (InferredTy::Variable(break_ty), target)
                } else {
                    // no break, loop is an infinite loop, meaning the end_block is
                    // now dangling. Create an undefined instruction so we have a target
                    let target = mir.new_local(Ty::ERROR);
                    mir.push_instr(end_block, Instr::Undefined { target });
                    (InferredTy::Concrete(Ty::NEVER), target)
                };
                scopes.exit_loop_scope();
                (end_block, expr_ty, target)
            }
            ast::ExprData::Call(call_expr) => {
                let (return_block, return_ty) =
                    self.analyze_call(expr, block, mir, type_solver, scopes, call_expr);
                let target = type_solver.new_registered_local(mir, &return_ty);
                mir.push_instr(return_block, Instr::ReturnValue { target });
                (return_block, return_ty, target)
            }
            ast::ExprData::Paren(expr) => self.analyze_expr(expr, block, mir, type_solver, scopes),
        }
    }

    fn analyze_call(
        &mut self,
        expr: &ast::Expr,
        block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
        call_expr: &ast::Call,
    ) -> (BlockId, InferredTy) {
        if scopes.const_decl_scope().is_some() {
            self.result_builder
                .add_error(CallNotSupportInConstDeclError { span: expr.span });
        }
        let (block, callee_ty, callee) =
            self.analyze_place_expr(&call_expr.callee, block, mir, type_solver, scopes);

        let return_ty = type_solver.check_call_expr(call_expr, expr.span, callee_ty);

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
        (return_block, return_ty)
    }

    fn analyze_place_expr(
        &mut self,
        place: &ast::Place,
        block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, InferredTy, Operand) {
        let (block, place_ty, resolved_place) =
            self.resolve_place(place, block, mir, type_solver, scopes);
        let resolved_place = match resolved_place {
            Some(resolved_place) => resolved_place,
            None => {
                let target = mir.new_local(Ty::ERROR);
                mir.push_instr(block, Instr::Undefined { target });
                return (block, place_ty, Operand::Local(target));
            }
        };
        match resolved_place {
            ResolvedPlace::Function(fn_ptr) => (block, place_ty, Operand::FnPtr(fn_ptr)),
            ResolvedPlace::Variable(variable, _, projections) => {
                let source = mir.read_variable(variable, block);
                let target = if projections.is_empty() {
                    source
                } else {
                    let target = type_solver.new_registered_local(mir, &place_ty);
                    mir.push_instr(
                        block,
                        Instr::Read {
                            target,
                            source,
                            projections,
                        },
                    );
                    target
                };
                (block, place_ty, Operand::Local(target))
            }
            ResolvedPlace::Global(source, projections) => {
                let target = type_solver.new_registered_local(mir, &place_ty);
                mir.push_instr(
                    block,
                    Instr::Load {
                        target,
                        source,
                        projections,
                    },
                );
                (block, place_ty, Operand::Local(target))
            }
        }
    }

    fn resolve_place(
        &mut self,
        place: &ast::Place,
        mut block: BlockId,
        mir: &mut FunctionBuilder,
        type_solver: &mut TypeSolver,
        scopes: &mut Scopes,
    ) -> (BlockId, InferredTy, Option<ResolvedPlace>) {
        let (place_ty, mut place_span, place_id) = match scopes.get_place(&place.name.data) {
            Some(place_scope) => (place_scope.ty.clone(), place.name.span, place_scope.mir_id),
            None => {
                self.result_builder
                    .add_error(NotDefinedError::from(place.name.clone()));
                return (block, InferredTy::Concrete(Ty::ERROR), None);
            }
        };
        if let PlaceId::Global(global) = place_id
            && let Some(const_decl_scope) = scopes.const_decl_scope()
        {
            if self.mir_builder.is_mutable_global(global) {
                self.result_builder
                    .add_error(GlobalReferencedInConstDeclError::from(place.name.clone()));
            } else {
                if !const_decl_scope.used_const_decls.contains_key(global) {
                    const_decl_scope.used_const_decls.insert(global, place.span);
                }
            }
        }
        let mut projections = Vec::new();
        let mut projected_ty = place_ty.clone();
        for part in &place.projections {
            let (new_projected_ty, new_place_span, new_block, projection) = match part {
                ast::PlaceProjection::Field(field) => {
                    let field_ty = type_solver.check_field_access(field, place_span, projected_ty);
                    (
                        field_ty,
                        field.span,
                        block,
                        PlaceProjection::Field(field.data.clone()),
                    )
                }
                ast::PlaceProjection::Index(index) => {
                    let (block, index_expr_ty, target) =
                        self.analyze_expr(index, block, mir, type_solver, scopes);
                    let item_ty = type_solver.check_array_index(
                        index,
                        place_span,
                        projected_ty,
                        index_expr_ty,
                    );
                    (
                        item_ty,
                        index.span,
                        block,
                        PlaceProjection::Index(Operand::Local(target)),
                    )
                }
            };
            projections.push(projection);
            projected_ty = new_projected_ty;
            place_span = new_place_span;
            block = new_block;
        }
        let resolved_place = match place_id {
            PlaceId::Function(function) if projections.is_empty() => {
                Some(ResolvedPlace::Function(function))
            }
            PlaceId::Function(_) => None,
            PlaceId::Variable(variable) => {
                Some(ResolvedPlace::Variable(variable, place_ty, projections))
            }
            PlaceId::Global(global) => Some(ResolvedPlace::Global(global, projections)),
        };
        (block, projected_ty, resolved_place)
    }
}

#[derive(Default)]
struct Scopes {
    fn_scope: Option<FnScope>,
    const_decl_scope: Option<ConstDeclScope>,
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

    fn enter_block_scope(&mut self, span: SourceSpan, yield_ty: Option<TyVar>) {
        assert!(!self.place_scopes.is_empty(), "not in global scope");
        self.block_scopes.push(Some(BlockScope {
            span,
            yield_ty,
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

    fn enter_loop_scope(
        &mut self,
        span: SourceSpan,
        break_var: Option<(TyVar, Variable)>,
        start_block: BlockId,
        end_block: BlockId,
    ) {
        self.loop_scopes.push(LoopScope {
            span,
            break_found: false,
            break_var,
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

    fn enter_fn_scope(&mut self, return_ty: Ty, span: SourceSpan, return_local: Option<LocalId>) {
        assert!(self.fn_scope.is_none(), "already in fn scope");
        assert!(
            self.const_decl_scope.is_none(),
            "already in const decl scope"
        );
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

    fn const_decl_scope(&mut self) -> Option<&mut ConstDeclScope> {
        self.const_decl_scope.as_mut()
    }

    fn enter_const_decl_scope(&mut self) {
        assert!(
            self.const_decl_scope.is_none(),
            "already in const decl scope"
        );
        assert!(self.fn_scope.is_none(), "already in fn scope");
        assert_eq!(self.place_scopes.len(), 1, "missing global place scope");
        self.const_decl_scope = Some(ConstDeclScope {
            used_const_decls: SparseSecondaryMap::new(),
        });
        self.place_scopes.push(HashMap::new());
        self.block_scopes.push(None);
    }

    fn exit_const_decl_scope(&mut self) {
        assert!(self.const_decl_scope.is_some(), "not in const decl scope");
        assert_eq!(self.block_scopes.len(), 1, "unbalanced block enters/exits");
        assert!(self.loop_scopes.is_empty(), "unbalanced loop enters/exits");
        assert_eq!(self.place_scopes.len(), 2, "unbalanced place enters/exits");
        self.const_decl_scope = None;
        let option = self.place_scopes.pop();
        debug_assert!(option.is_some(), "not in place scope");
        let option = self.block_scopes.pop();
        debug_assert!(
            option.is_some() && option.unwrap().is_none(),
            "not in const decl scope"
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
    Variable(Variable, InferredTy, Vec<PlaceProjection>),
    Global(GlobalId, Vec<PlaceProjection>),
}

struct AnalyzedGlobalScope<'ast> {
    scopes: Scopes,
    fn_decls: IndexMap<FunctionId, (&'ast ast::FnDecl, Ty, FnDeclAttributes)>,
    const_decls: IndexMap<GlobalId, (&'ast ast::ConstDecl, Ty)>,
}

struct FnScope {
    return_ty: Ty,
    span: SourceSpan,
    return_local: Option<LocalId>,
}

struct ConstDeclScope {
    used_const_decls: SparseSecondaryMap<GlobalId, SourceSpan>,
}

struct BlockScope {
    span: SourceSpan,
    yield_ty: Option<TyVar>,
    yield_var: Option<Variable>,
}

struct LoopScope {
    break_var: Option<(TyVar, Variable)>,
    break_found: bool,
    span: SourceSpan,
    start_block: BlockId,
    end_block: BlockId,
}

#[derive(PartialEq, Clone, Copy)]
enum StatementTy {
    Never,
    Unit,
}

impl From<&InferredTy> for StatementTy {
    fn from(value: &InferredTy) -> Self {
        if value == &InferredTy::Concrete(Ty::NEVER) {
            StatementTy::Never
        } else {
            StatementTy::Unit
        }
    }
}
