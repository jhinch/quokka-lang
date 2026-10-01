use super::InstrCtx;
use crate::lir::{BlockId, Terminator};
use crate::mir;

impl InstrCtx<'_> {
    pub(super) fn generate_from_terminator(
        &mut self,
        terminator: &mir::Terminator,
        block: BlockId,
    ) {
        match terminator {
            mir::Terminator::Return => {
                self.lir.set_terminator(block, Terminator::Ret);
            }
            mir::Terminator::Call {
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
            } => {
                let callee = self.functions[*callee];
                let return_block = self.blocks[*mir_return_block];
                self.lir.set_terminator(
                    block,
                    Terminator::Call {
                        callee,
                        return_block,
                    },
                );
            }
            mir::Terminator::Call {
                callee: mir::Callee::Local(callee),
                return_block: mir_return_block,
            } => {
                let (callee_high, callee_low) = self.locals.r16s[*callee];
                let callee = self.lir.insert_join(block, callee_high, callee_low);
                let return_block = self.blocks[*mir_return_block];
                self.lir.set_terminator(
                    block,
                    Terminator::CallR16 {
                        callee,
                        return_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                source: mir_source,
                condition: mir_condition,
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.locals.r1s[*mir_source];
                let (source, condition) = self.r1_to_flag(source, condition, block);
                let callee = self.functions[*callee];
                let return_block = self.blocks[*mir_return_block];
                let else_block = self.blocks[*mir_else_block];
                self.lir.set_terminator(
                    block,
                    Terminator::CallIf {
                        source,
                        negate: *mir_condition != condition,
                        callee,
                        return_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                callee: mir::Callee::Local(_),
                ..
            } => unimplemented!(
                "{terminator:?} as it should be rewritten within the optimization phase"
            ),
            mir::Terminator::Jump {
                target_block: mir_target_block,
            } => {
                let target_block = self.blocks[*mir_target_block];
                self.lir.set_terminator(
                    block,
                    Terminator::Jp {
                        next_block: target_block,
                    },
                );
            }
            mir::Terminator::JumpIf {
                source: mir_source,
                condition: mir_condition,
                then_block: mir_then_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.locals.r1s[*mir_source];
                let (source, condition) = self.r1_to_flag(source, condition, block);
                let then_block = self.blocks[*mir_then_block];
                let else_block = self.blocks[*mir_else_block];
                self.lir.set_terminator(
                    block,
                    Terminator::JpIf {
                        source,
                        negate: *mir_condition != condition,
                        then_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::ReturnIf {
                source: mir_source,
                condition: mir_condition,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.locals.r1s[*mir_source];
                let (source, condition) = self.r1_to_flag(source, condition, block);
                let else_block = self.blocks[*mir_else_block];
                self.lir.set_terminator(
                    block,
                    Terminator::RetIf {
                        source,
                        negate: *mir_condition != condition,
                        else_block,
                    },
                );
            }
        }
    }
}
