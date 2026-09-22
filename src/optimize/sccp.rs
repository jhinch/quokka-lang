use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{
    BlockId, Condition, Function, FunctionMutator, Global, GlobalId, Instr, LocalId, Operand,
    PlaceProjection, Terminator,
};
use crate::optimize::{Optimize, Outcome};
use crate::ty::{Ty, TyRegistry};
use quokka_lang_slotkeyset::SlotKeySet;
use slotmap::sparse_secondary::Entry;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap};
use std::collections::VecDeque;
use tracing::instrument;

/// Sparse Conditional Constant Propagation
///
/// Original algorithm
/// from ("Constant Propagation with Conditional Branches", 1991)
/// by Mark N. Wegman and F. Kenneth Zadeck
///
/// Algorithm pseudocode is
/// from ("Engineering a Compiler", 2004)
/// Figure 10.14
/// by Keith D. Cooper & Linda Torczon
///
/// The original algorithm proposes tracking each phi edge and filtering the sources
/// when evaluating its value. However, this is unnecessary bookkeeping, and instead we
/// exploit that each unvisited previous block source will evaluate to Top which is the
/// same as filtering them out.
///
pub struct Sccp;

impl Optimize for Sccp {
    #[instrument(name = "sccp", skip_all, fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(
        &self,
        function: &mut Function,
        globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
    ) -> Outcome {
        let evaluation = self.solve(function, globals, ty_registry);
        self.propagate(function, evaluation)
    }
}

impl Sccp {
    fn solve(
        &self,
        function: &Function,
        globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
    ) -> Solution {
        let uses = self.calculate_uses(function);
        let mut values =
            SecondaryMap::<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>::with_capacity(
                function.locals.len(),
            );
        let mut required_blocks = SlotKeySet::<BlockId>::with_capacity(function.blocks.len());
        let mut cfg_work_queue = VecDeque::new();
        let mut ssa_work_queue = VecDeque::new();

        cfg_work_queue.push_back(CfgWork(function.start_block));
        while !cfg_work_queue.is_empty() || !ssa_work_queue.is_empty() {
            while let Some(CfgWork(block_id)) = cfg_work_queue.pop_front() {
                if !required_blocks.insert(block_id) {
                    continue;
                }
                let block = &function.blocks[block_id];
                for instruction in &block.instructions {
                    self.visit_instruction(
                        instruction,
                        block_id,
                        function,
                        globals,
                        ty_registry,
                        &mut values,
                        &mut ssa_work_queue,
                    );
                }
                self.visit_terminator(&block.terminator, &values, &mut cfg_work_queue);
            }
            while let Some(SSAWork(local_id)) = ssa_work_queue.pop_front() {
                for local_use in uses.get(local_id).into_iter().flatten().copied() {
                    match local_use {
                        Use::Instr(block_id, index) => {
                            if required_blocks.contains(block_id) {
                                self.visit_instruction(
                                    &function.blocks[block_id].instructions[index],
                                    block_id,
                                    function,
                                    globals,
                                    ty_registry,
                                    &mut values,
                                    &mut ssa_work_queue,
                                );
                            }
                        }
                        Use::Branch(block_id) => {
                            if required_blocks.contains(block_id) {
                                self.visit_branch(
                                    &function.blocks[block_id].terminator,
                                    &values,
                                    &mut cfg_work_queue,
                                );
                            }
                        }
                    }
                }
            }
        }
        Solution {
            values,
            required_blocks,
        }
    }

    fn calculate_uses(&self, function: &Function) -> SecondaryMap<LocalId, Vec<Use>> {
        let mut uses = SecondaryMap::<LocalId, Vec<Use>>::new();
        for (block_id, block) in &function.blocks {
            for (index, instruction) in block.instructions.iter().enumerate() {
                for source_id in instruction.source_locals() {
                    uses.entry(source_id)
                        .unwrap()
                        .or_default()
                        .push(Use::Instr(block_id, index));
                }
            }
            for source_id in block.terminator.source_locals() {
                uses.entry(source_id)
                    .unwrap()
                    .or_default()
                    .push(Use::Branch(block_id));
            }
        }
        uses
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_instruction(
        &self,
        instruction: &Instr,
        block_id: BlockId,
        function: &Function,
        globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
        values: &mut SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
        ssa_work_queue: &mut VecDeque<SSAWork>,
    ) {
        let Some(target) = instruction.target_local() else {
            return;
        };
        let value = self.model_instruction(instruction, function, globals, ty_registry, values);
        let changed = match values
            .entry(target)
            .unwrap()
            .or_default()
            .entry(block_id)
            .unwrap()
        {
            Entry::Occupied(entry) if entry.get() == &ResolvedValue::Bottom => false,
            Entry::Occupied(entry) if entry.get() == &value => false,
            Entry::Occupied(mut entry) => {
                tracing::trace!("target {target:?} in block {block_id:?} resolved to {value:?}");
                entry.insert(value);
                true
            }
            Entry::Vacant(entry) => {
                tracing::trace!("target {target:?} in block {block_id:?} resolved to {value:?}");
                entry.insert(value);
                true
            }
        };
        if changed {
            ssa_work_queue.push_back(SSAWork(target));
        }
    }

    fn visit_terminator(
        &self,
        terminator: &Terminator,
        values: &SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
        cfg_work_queue: &mut VecDeque<CfgWork>,
    ) {
        match terminator {
            Terminator::CallIf { .. } | Terminator::JumpIf { .. } | Terminator::ReturnIf { .. } => {
                self.visit_branch(terminator, values, cfg_work_queue);
            }
            Terminator::Return => {}
            Terminator::Call {
                return_block: target_block,
                ..
            }
            | Terminator::Jump { target_block } => {
                cfg_work_queue.push_back(CfgWork(*target_block));
            }
        }
    }

    fn visit_branch(
        &self,
        terminator: &Terminator,
        values: &SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
        cfg_work_queue: &mut VecDeque<CfgWork>,
    ) {
        match terminator {
            Terminator::CallIf {
                source,
                condition,
                return_block: then_block,
                else_block,
                ..
            }
            | Terminator::JumpIf {
                source,
                condition,
                then_block,
                else_block,
            } => {
                let expected = match condition {
                    Condition::NotZero => Value::U1(true),
                    Condition::Zero => Value::U1(false),
                };
                let value = self.resolve_local(*source, values);
                if value.meet(&ResolvedValue::Value(!expected.clone())) == &ResolvedValue::Bottom {
                    tracing::trace!(
                        "block terminator resolved to {condition:?} so visiting edge {then_block:?}"
                    );
                    cfg_work_queue.push_back(CfgWork(*then_block));
                }
                if value.meet(&ResolvedValue::Value(expected.clone())) == &ResolvedValue::Bottom {
                    tracing::trace!(
                        "block terminator resolved to {condition:?} so visiting edge {else_block:?}"
                    );
                    cfg_work_queue.push_back(CfgWork(*else_block));
                }
            }
            Terminator::ReturnIf {
                source,
                condition,
                else_block,
            } => {
                let expected = match condition {
                    Condition::NotZero => Value::U1(true),
                    Condition::Zero => Value::U1(false),
                };
                let value = self.resolve_local(*source, values);
                if value.meet(&ResolvedValue::Value(expected.clone())) == &ResolvedValue::Bottom {
                    tracing::trace!(
                        "block terminator resolved to {condition:?} so visiting edge {else_block:?}"
                    );
                    cfg_work_queue.push_back(CfgWork(*else_block));
                }
            }
            _ => unimplemented!("{terminator:?} is not a branch"),
        }
    }

    fn model_instruction(
        &self,
        instr: &Instr,
        function: &Function,
        globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
        values: &SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
    ) -> ResolvedValue {
        match instr {
            Instr::Nop | Instr::Store { .. } | Instr::Transfer { .. } | Instr::Fill { .. } => {
                unimplemented!("instruction {instr:?} does not produce a value")
            }
            Instr::Undefined { .. } => ResolvedValue::Bottom,
            Instr::Alloc { target } => {
                let target_ty = function.locals[*target].ty;
                ResolvedValue::Value(Value::zeros(target_ty, ty_registry))
            }
            Instr::ReturnValue { .. } => ResolvedValue::Bottom,
            Instr::Phi { sources, .. } => sources
                .values()
                .map(|&source_id| self.resolve_local(source_id, values))
                .reduce(ResolvedValue::meet)
                .unwrap_or(&ResolvedValue::Top)
                .clone(),
            Instr::Load {
                source,
                projections,
                ..
            } => {
                let global = &globals[*source];
                // we should be able to resolve constants for projections as well, but it would make
                // sense to refactor more out of
                if !global.is_mutable() && projections.is_empty() {
                    ResolvedValue::Value(Value::from_bytes(
                        global.ty,
                        ty_registry,
                        global.value.as_slice(),
                    ))
                } else {
                    ResolvedValue::Bottom
                }
            }
            Instr::Copy { target, source } => {
                let source_ty = function.locals[*target].ty;
                self.resolve_operand(source, source_ty, ty_registry, values)
            }
            // for now, reads and writes are Bottom even though they can be implemented
            Instr::Read { .. } => ResolvedValue::Bottom,
            Instr::Write { .. } => ResolvedValue::Bottom,
            Instr::BitCast { target, source } => {
                let target_ty = function.locals[*target].ty;
                match self.resolve_local(*source, values).clone() {
                    ResolvedValue::Value(value) => {
                        let bytes = value.into_vec();
                        ResolvedValue::Value(Value::from_bytes(
                            target_ty,
                            ty_registry,
                            bytes.as_slice(),
                        ))
                    }
                    value => value,
                }
            }
            Instr::Not { source, .. } => match self.resolve_local(*source, values).clone() {
                ResolvedValue::Value(value) => ResolvedValue::Value(!value),
                value => value,
            },
            Instr::Neg { source, .. } => match self.resolve_local(*source, values).clone() {
                ResolvedValue::Value(value) => ResolvedValue::Value(-value),
                value => value,
            },
            Instr::RotateLeft { source, .. } => match self.resolve_local(*source, values).clone() {
                ResolvedValue::Value(value) => ResolvedValue::Value(value.rotate_left_one()),
                value => value,
            },
            Instr::RotateRight { source, .. } => {
                match self.resolve_local(*source, values).clone() {
                    ResolvedValue::Value(value) => ResolvedValue::Value(value.rotate_right_one()),
                    value => value,
                }
            }
            Instr::ShiftLeft { source, .. } => match self.resolve_local(*source, values).clone() {
                ResolvedValue::Value(value) => ResolvedValue::Value(value.shift_left_one()),
                value => value,
            },
            Instr::ShiftRight { source, .. } => match self.resolve_local(*source, values).clone() {
                ResolvedValue::Value(value) => ResolvedValue::Value(value.shift_right_one()),
                value => value,
            },
            Instr::Eq { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        ResolvedValue::Value(Value::U1(left == &right))
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::Ne { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        ResolvedValue::Value(Value::U1(left != &right))
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::Lt { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        ResolvedValue::Value(Value::U1(left < &right))
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::Ge { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        ResolvedValue::Value(Value::U1(left >= &right))
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::BitAnd { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        let mut result = left.clone();
                        result &= right;
                        ResolvedValue::Value(result)
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::BitOr { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        let mut result = left.clone();
                        result |= right;
                        ResolvedValue::Value(result)
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::BitXor { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        let mut result = left.clone();
                        result ^= right;
                        ResolvedValue::Value(result)
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::Add { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        let mut result = left.clone();
                        result += right;
                        ResolvedValue::Value(result)
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
            Instr::Sub { left, right, .. } => {
                let resolved = self.resolve_binary(
                    *left,
                    right,
                    function.locals[*left].ty,
                    ty_registry,
                    values,
                );
                match resolved {
                    ResolvedValue::Value((left, right)) => {
                        let mut result = left.clone();
                        result -= right;
                        ResolvedValue::Value(result)
                    }
                    ResolvedValue::Bottom => ResolvedValue::Bottom,
                    ResolvedValue::Top => ResolvedValue::Top,
                }
            }
        }
    }

    fn resolve_local<'a>(
        &self,
        source: LocalId,
        values: &'a SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
    ) -> &'a ResolvedValue {
        values
            .get(source)
            .and_then(|v| v.values().next())
            .unwrap_or(&ResolvedValue::Top)
    }

    fn resolve_operand(
        &self,
        source: &Operand,
        source_ty: Ty,
        ty_registry: &TyRegistry,
        values: &SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
    ) -> ResolvedValue {
        match source {
            Operand::Local(source) => self.resolve_local(*source, values).clone(),
            Operand::Literal(value) => {
                let bytes = source_ty.bit_size(ty_registry).padded_bytes();
                ResolvedValue::Value(Value::from_bytes(
                    source_ty,
                    ty_registry,
                    &value.value.to_le_bytes()[..bytes as usize],
                ))
            }
            Operand::FnPtr(_) => ResolvedValue::Bottom,
        }
    }

    fn resolve_binary<'a>(
        &self,
        left: LocalId,
        right: &Operand,
        right_ty: Ty,
        ty_registry: &TyRegistry,
        values: &'a SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
    ) -> ResolvedValue<(&'a Value, Value)> {
        let left = self.resolve_local(left, values);
        let right = self.resolve_operand(right, right_ty, ty_registry, values);
        match (left, right) {
            (ResolvedValue::Bottom, _) | (_, ResolvedValue::Bottom) => ResolvedValue::Bottom,
            (ResolvedValue::Value(left), ResolvedValue::Value(right)) => {
                ResolvedValue::Value((left, right))
            }
            _ => ResolvedValue::Top,
        }
    }

    fn propagate(&self, function: &mut Function, evaluation: Solution) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for block_id in function.blocks.keys().collect::<Vec<_>>() {
            if !evaluation.required_blocks.contains(block_id) {
                continue; // allow to be cleaned up by remove_dangling
            }
            for instruction in function.blocks[block_id].instructions.iter_mut() {
                if let Some(target) = instruction.target_local() {
                    let value = evaluation
                        .values
                        .get(target)
                        .and_then(|values| values.get(block_id))
                        .unwrap_or(&ResolvedValue::Bottom);
                    if let ResolvedValue::Value(value) = value
                        && !matches!(
                            instruction,
                            Instr::Copy {
                                source: Operand::Literal(_),
                                ..
                            } | Instr::Alloc { .. }
                        )
                    {
                        let mut bytes = value.clone().into_vec();
                        // in practice, you can never have bytes > 2 as runtime
                        // local variables are constrained to at most 2 bytes
                        // and const declarations are not run through an optimization
                        // pass. We check for 4 as currently IntegerLiteral uses u32 (4 bytes)
                        if bytes.len() <= 4 {
                            bytes.resize(4, 0);
                            let bytes = *bytes.as_array::<4>().unwrap();
                            *instruction = Instr::Copy {
                                target,
                                source: Operand::Literal(IntegerLiteral::from(u32::from_le_bytes(
                                    bytes,
                                ))),
                            };
                            outcome = Outcome::Modified;
                        }
                    }
                }
                match instruction {
                    Instr::Load { projections, .. }
                    | Instr::Store { projections, .. }
                    | Instr::Read { projections, .. }
                    | Instr::Write { projections, .. } => {
                        for projection in projections.iter_mut() {
                            let PlaceProjection::Index(index) = projection else {
                                continue;
                            };
                            if let Operand::Local(index_id) = index {
                                let index_id = *index_id;
                                let index_value = evaluation
                                    .values
                                    .get(index_id)
                                    .and_then(|values| values.get(block_id))
                                    .unwrap_or(&ResolvedValue::Bottom);
                                if let ResolvedValue::Value(index_value) = index_value {
                                    let mut bytes = index_value.clone().into_vec();
                                    bytes.resize(4, 0);
                                    let bytes = *bytes.as_array::<4>().unwrap();
                                    *index = Operand::Literal(IntegerLiteral::from(
                                        u32::from_le_bytes(bytes),
                                    ));
                                    outcome = Outcome::Modified;
                                }
                            }
                        }
                        match instruction {
                            Instr::Store { source, .. } | Instr::Write { right: source, .. } => {
                                if let Operand::Local(source_id) = source {
                                    let source_id = *source_id;
                                    let source_value = evaluation
                                        .values
                                        .get(source_id)
                                        .and_then(|values| values.get(block_id))
                                        .unwrap_or(&ResolvedValue::Bottom);
                                    if let ResolvedValue::Value(source_value) = source_value {
                                        let mut bytes = source_value.clone().into_vec();
                                        bytes.resize(4, 0);
                                        let bytes = *bytes.as_array::<4>().unwrap();
                                        *source = Operand::Literal(IntegerLiteral::from(
                                            u32::from_le_bytes(bytes),
                                        ));
                                        outcome = Outcome::Modified;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Instr::Eq { right, .. }
                    | Instr::Ne { right, .. }
                    | Instr::Lt { right, .. }
                    | Instr::Ge { right, .. }
                    | Instr::BitAnd { right, .. }
                    | Instr::BitOr { right, .. }
                    | Instr::BitXor { right, .. }
                    | Instr::Add { right, .. }
                    | Instr::Sub { right, .. } => {
                        if let Operand::Local(right_id) = right {
                            let right_id = *right_id;
                            let right_value = evaluation
                                .values
                                .get(right_id)
                                .and_then(|values| values.get(block_id))
                                .unwrap_or(&ResolvedValue::Bottom);
                            if let ResolvedValue::Value(right_value) = right_value {
                                let mut bytes = right_value.clone().into_vec();
                                bytes.resize(4, 0);
                                let bytes = *bytes.as_array::<4>().unwrap();
                                *right = Operand::Literal(IntegerLiteral::from(
                                    u32::from_le_bytes(bytes),
                                ));
                                outcome = Outcome::Modified;
                            }
                        }
                    }
                    _ => {}
                }
            }
            for next_block_id in function.blocks[block_id].next_blocks().collect::<Vec<_>>() {
                if !evaluation.required_blocks.contains(next_block_id) {
                    FunctionMutator::from(&mut *function)
                        .remove_next_block(block_id, next_block_id);
                    outcome = Outcome::Modified;
                }
            }
        }
        if outcome == Outcome::Modified {
            function.remove_dangling();
            #[cfg(debug_assertions)]
            function.check_integrity().unwrap();
        }
        outcome
    }
}
struct Solution {
    values: SecondaryMap<LocalId, SparseSecondaryMap<BlockId, ResolvedValue>>,
    required_blocks: SlotKeySet<BlockId>,
}

pub struct CfgWork(BlockId);
pub struct SSAWork(LocalId);

#[derive(Debug, Clone, Copy)]
pub enum Use {
    Instr(BlockId, usize),
    Branch(BlockId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedValue<T = Value> {
    /// Top = unresolved
    Top,
    Value(T),
    /// Bottom = must be commuted at runtime
    Bottom,
}

impl ResolvedValue {
    /// self /\ other
    pub fn meet<'a>(&'a self, other: &'a ResolvedValue) -> &'a ResolvedValue {
        match (self, other) {
            (ResolvedValue::Top, value) | (value, ResolvedValue::Top) => value,
            (ResolvedValue::Value(lhs), ResolvedValue::Value(rhs)) if lhs == rhs => self,
            _ => &ResolvedValue::Bottom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Identifier, SpannedIdentifier};
    use crate::attr::GlobalDeclAttributes;
    use crate::mir::{FunctionBuilder, GlobalAttributes};
    use miette::SourceSpan;

    fn new_builder(return_ty: Ty) -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from("f"), return_ty)
    }

    fn run_sccp(
        function: &mut Function,
        globals: &SlotMap<GlobalId, Global>,
        ty_registry: &TyRegistry,
    ) -> Outcome {
        Sccp.optimize_function(function, globals, ty_registry)
    }

    fn literal(target: LocalId, value: u32) -> Instr {
        Instr::Copy {
            target,
            source: Operand::Literal(IntegerLiteral::from(value)),
        }
    }

    fn ident(name: &str) -> SpannedIdentifier {
        SpannedIdentifier::new(name, SourceSpan::new(0.into(), 0))
    }

    // --- straight-line constant folding ---

    #[test]
    fn folds_constant_addition_into_a_literal_copy() {
        let mut fb = new_builder(Ty::U8);
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let a = fb.new_local(Ty::U8);
        let b = fb.new_local(Ty::U8);
        fb.push_instr(block, literal(a, 3));
        fb.push_instr(block, literal(b, 4));
        fb.push_instr(
            block,
            Instr::Add {
                target: ret,
                left: a,
                right: Operand::Local(b),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();
        let globals = SlotMap::with_key();
        let ty_registry = TyRegistry::default();

        let outcome = run_sccp(&mut function, &globals, &ty_registry);

        assert_eq!(outcome, Outcome::Modified);
        assert!(matches!(
            function.blocks[block].instructions[2],
            Instr::Copy {
                source: Operand::Literal(IntegerLiteral { value: 7 }),
                ..
            }
        ));
        function.check_integrity().unwrap();
    }

    // --- branch pruning once the condition resolves to a compile-time constant ---

    #[test]
    fn constant_condition_prunes_the_unreachable_edge() {
        let mut fb = new_builder(Ty::U8);
        let b1 = fb.new_block();
        let b2 = fb.new_block(); // taken (cond is always true)
        let b3 = fb.new_block(); // unreachable
        let ret = fb.create_return_local(Ty::U8);
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(b1, literal(cond, 1));
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        fb.push_instr(b2, literal(ret, 1));
        fb.set_terminator(b2, Terminator::Return);
        fb.push_instr(b3, literal(ret, 2));
        fb.set_terminator(b3, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        let mut function = fb.build();
        let globals = SlotMap::with_key();
        let ty_registry = TyRegistry::default();

        let outcome = run_sccp(&mut function, &globals, &ty_registry);

        assert_eq!(outcome, Outcome::Modified);
        assert!(
            matches!(
                function.blocks[b1].terminator,
                Terminator::Jump { target_block } if target_block == b2
            ),
            "cond is always true, so b1 must become an unconditional jump to b2"
        );
        assert!(
            !function.blocks.contains_key(b3),
            "the unreachable else arm must be dropped"
        );
        function.check_integrity().unwrap();
    }

    // --- phi meet lattice ---

    #[test]
    fn phi_folds_to_a_literal_when_every_reachable_arm_agrees() {
        let mut fb = new_builder(Ty::U8);
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let b4 = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(b1, literal(cond, 0));
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        let x2 = fb.new_local(Ty::U8);
        fb.push_instr(b2, literal(x2, 5));
        fb.set_terminator(b2, Terminator::Jump { target_block: b4 });
        let x3 = fb.new_local(Ty::U8);
        fb.push_instr(b3, literal(x3, 5));
        fb.set_terminator(b3, Terminator::Jump { target_block: b4 });
        let mut sources = SparseSecondaryMap::new();
        sources.insert(b2, x2);
        sources.insert(b3, x3);
        fb.push_instr(
            b4,
            Instr::Phi {
                target: ret,
                sources,
            },
        );
        fb.set_terminator(b4, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        fb.seal_block(b4);
        let mut function = fb.build();
        let globals = SlotMap::with_key();
        let ty_registry = TyRegistry::default();

        run_sccp(&mut function, &globals, &ty_registry);

        assert!(
            matches!(
                function.blocks[b4].instructions[0],
                Instr::Copy {
                    source: Operand::Literal(IntegerLiteral { value: 5 }),
                    ..
                }
            ),
            "both arms agree on 5, so the phi should fold to a literal"
        );
    }

    #[test]
    fn phi_stays_unresolved_when_reachable_arms_disagree() {
        let mut globals = SlotMap::with_key();
        let cond_global = globals.insert(Global {
            ty: Ty::U1,
            name: ident("COND"),
            attrs: GlobalAttributes::Ram(GlobalDeclAttributes::default()),
            value: Vec::new(),
        });

        let mut fb = new_builder(Ty::U8);
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let b4 = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        // condition is loaded from a *mutable* global, so SCCP cannot resolve it to a
        // compile-time constant and both arms stay reachable
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(
            b1,
            Instr::Load {
                target: cond,
                source: cond_global,
                projections: Vec::new(),
            },
        );
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        let x2 = fb.new_local(Ty::U8);
        fb.push_instr(b2, literal(x2, 5));
        fb.set_terminator(b2, Terminator::Jump { target_block: b4 });
        let x3 = fb.new_local(Ty::U8);
        fb.push_instr(b3, literal(x3, 6));
        fb.set_terminator(b3, Terminator::Jump { target_block: b4 });
        let mut sources = SparseSecondaryMap::new();
        sources.insert(b2, x2);
        sources.insert(b3, x3);
        fb.push_instr(
            b4,
            Instr::Phi {
                target: ret,
                sources,
            },
        );
        fb.set_terminator(b4, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        fb.seal_block(b4);
        let mut function = fb.build();
        let ty_registry = TyRegistry::default();

        run_sccp(&mut function, &globals, &ty_registry);

        assert!(
            matches!(function.blocks[b4].instructions[0], Instr::Phi { .. }),
            "since cond isn't itself constant, both 5 and 6 are reachable, so the phi cannot fold"
        );
    }
}
