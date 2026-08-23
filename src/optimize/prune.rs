use crate::mir::{Function, FunctionMutator, Instr, Terminator};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use tracing::instrument;

/// remove nop instructions and join empty blocks
pub struct Prune;

impl Optimize for Prune {
    #[instrument(skip(self, function, _ty_registry), fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(&self, function: &mut Function, _ty_registry: &TyRegistry) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for mut block_id in function.traverse_blocks().collect::<Vec<_>>() {
            let mut block_outcome = Outcome::Unmodified;
            let mut clean_count = 0;
            function.blocks[block_id].instructions.retain(|instr| {
                if matches!(instr, Instr::Nop) {
                    block_outcome = Outcome::Modified;
                    false
                } else {
                    if block_outcome == Outcome::Unmodified {
                        clean_count += 1;
                    }
                    true
                }
            });

            let mut previous_blocks = function.blocks[block_id].previous_blocks();
            if let Some(previous_block_id) = previous_blocks.next()
                && previous_blocks.count() == 0
            {
                match (
                    &function.blocks[previous_block_id].terminator,
                    function.blocks[block_id].instructions.len(),
                    &function.blocks[block_id].terminator,
                ) {
                    (Terminator::Jump { .. }, _, _) => {
                        FunctionMutator::from(&mut *function)
                            .join_blocks(previous_block_id, block_id);
                        block_id = previous_block_id;
                        clean_count = function.blocks[block_id].instructions.len();
                        block_outcome = Outcome::Modified;
                    }
                    (_, 0, Terminator::Jump { target_block }) => {
                        let target_block = *target_block;
                        FunctionMutator::from(&mut *function).replace_next_block(
                            previous_block_id,
                            block_id,
                            target_block,
                        );
                        block_outcome = Outcome::Modified;
                    }
                    (Terminator::ReturnIf { .. }, 0, Terminator::Return) => {
                        FunctionMutator::from(&mut *function)
                            .remove_next_block(previous_block_id, block_id);
                        block_outcome = Outcome::Modified;
                    }
                    _ => {}
                }
            }

            // do this after joining blocks as it avoids unnecessary bookkeeping
            while clean_count < function.blocks[block_id].instructions.len() {
                if let Some(target) =
                    function.blocks[block_id].instructions[clean_count].target_local()
                {
                    function.locals[target]
                        .defined
                        .insert(block_id, clean_count);
                }
                clean_count += 1;
            }

            outcome |= block_outcome;
        }
        if outcome == Outcome::Modified {
            function.remove_dangling();
            // #[cfg(debug_assertions)]
            // function.check_integrity().unwrap();
        }
        outcome
    }
}
