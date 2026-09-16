use crate::mir::{
    Callee, FunctionId, GlobalAttributes, GlobalId, Instr, Mir, Operand, PlaceProjection,
    Terminator,
};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use quokka_lang_slotkeyset::SlotKeySet;
use std::collections::VecDeque;
use tracing::instrument;

/// Tree shaking pass
///
/// Tree shaking eliminates dead top level items including:
///
/// - Unused functions
/// - Unused globals (constant and mutable)
///
/// See [`super::adce`] for dead code elimination within functions
///
pub struct TreeShake;

impl Optimize for TreeShake {
    #[instrument(name = "tree_shake", skip_all, ret(level = tracing::Level::DEBUG))]
    fn optimize(&self, mir: &mut Mir, _ty_registry: &TyRegistry) -> Outcome {
        let (used_functions, used_globals) = self.calculate_used(mir);
        let function_count = mir.functions.len();
        let global_count = mir.globals.len();
        mir.functions.retain(|id, _| used_functions.contains(id));
        mir.globals.retain(|id, _| used_globals.contains(id));
        if mir.functions.len() != function_count || mir.globals.len() != global_count {
            Outcome::Modified
        } else {
            Outcome::Unmodified
        }
    }
}

impl TreeShake {
    fn calculate_used(&self, mir: &Mir) -> (SlotKeySet<FunctionId>, SlotKeySet<GlobalId>) {
        let mut used_functions = SlotKeySet::new();
        let mut used_globals = SlotKeySet::new();

        for (global_id, global) in &mir.globals {
            if let GlobalAttributes::Rom(attrs) = &global.attrs
                && attrs.address.is_some()
            {
                used_globals.insert(global_id);
            }
        }

        let mut queue = VecDeque::new();
        let Some(entrypoint) = mir.entrypoint else {
            return (used_functions, used_globals);
        };
        queue.push_back(entrypoint);
        used_functions.insert(entrypoint);
        while let Some(function_id) = queue.pop_front() {
            let function = &mir.functions[function_id];
            for block_id in function.traverse_blocks() {
                let block = &function.blocks[block_id];
                for instruction in &block.instructions {
                    match instruction {
                        Instr::Nop
                        | Instr::Undefined { .. }
                        | Instr::Alloc { .. }
                        | Instr::ReturnValue { .. }
                        | Instr::Phi { .. }
                        | Instr::Not { .. }
                        | Instr::Neg { .. }
                        | Instr::RotateLeft { .. }
                        | Instr::RotateRight { .. }
                        | Instr::ShiftLeft { .. }
                        | Instr::ShiftRight { .. } => {}
                        Instr::Load {
                            source: global,
                            projections,
                            ..
                        }
                        | Instr::Store {
                            target: global,
                            projections,
                            ..
                        } => {
                            used_globals.insert(*global);
                            for projection in projections {
                                if let PlaceProjection::Index(Operand::FnPtr(fn_ptr)) = projection
                                    && used_functions.insert(*fn_ptr)
                                {
                                    queue.push_back(*fn_ptr);
                                }
                            }
                        }
                        Instr::Read { projections, .. } | Instr::Write { projections, .. } => {
                            for projection in projections {
                                if let PlaceProjection::Index(Operand::FnPtr(fn_ptr)) = projection
                                    && used_functions.insert(*fn_ptr)
                                {
                                    queue.push_back(*fn_ptr);
                                }
                            }
                        }
                        Instr::Copy {
                            source: operand, ..
                        }
                        | Instr::Eq { right: operand, .. }
                        | Instr::Ne { right: operand, .. }
                        | Instr::Lt { right: operand, .. }
                        | Instr::Ge { right: operand, .. }
                        | Instr::BitAnd { right: operand, .. }
                        | Instr::BitOr { right: operand, .. }
                        | Instr::BitXor { right: operand, .. }
                        | Instr::Add { right: operand, .. }
                        | Instr::Sub { right: operand, .. } => {
                            if let Operand::FnPtr(fn_ptr) = operand
                                && used_functions.insert(*fn_ptr)
                            {
                                queue.push_back(*fn_ptr);
                            }
                        }
                    }
                }
                match &block.terminator {
                    Terminator::Return
                    | Terminator::Jump { .. }
                    | Terminator::JumpIf { .. }
                    | Terminator::ReturnIf { .. } => {}
                    Terminator::Call { callee, .. } | Terminator::CallIf { callee, .. } => {
                        if let Callee::FnPtr(callee) = callee
                            && used_functions.insert(*callee)
                        {
                            queue.push_back(*callee);
                        }
                    }
                }
            }
        }

        (used_functions, used_globals)
    }
}
