use crate::mir::{Function, FunctionMutator, Instr, Local, Operand, Terminator};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use slotmap::SparseSecondaryMap;
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
                        // the target block might have phi nodes.
                        // in that situation we will need to add
                        // a copy back into this block
                        // consider a block which all it does is decide for the
                        // subsequent phi node which local to use. This is ok because
                        // SCCP should push constants forwards which will allow for further
                        // simplication. Additionally, when lowered to LIR, copies in the
                        // preceding block will need to be added when resolving phi nodes
                        // anyway. The alternative would be that we never generate MIR
                        // without the copy already. For now, we will do it this way
                        let mut phi_sources =
                            if function.blocks[target_block].previous_blocks().count() > 1 {
                                function.blocks[target_block]
                                    .instructions
                                    .iter()
                                    .filter_map(|instruction| match instruction {
                                        Instr::Phi { target, sources } => {
                                            Some((*target, sources[block_id]))
                                        }
                                        _ => None,
                                    })
                                    .collect::<SparseSecondaryMap<_, _>>()
                            } else {
                                SparseSecondaryMap::new()
                            };
                        if !phi_sources.is_empty() {
                            for (index, (_, source)) in phi_sources.iter_mut().enumerate() {
                                let mut defined = SparseSecondaryMap::new();
                                defined.insert(block_id, index);
                                let target = function.locals.insert(Local {
                                    ty: function.locals[*source].ty,
                                    is_return: false,
                                    defined,
                                    name: function.locals[*source].name.clone(),
                                });
                                function.blocks[block_id].instructions.push(Instr::Copy {
                                    target,
                                    source: Operand::Local(*source),
                                });
                                *source = target;
                            }
                        } else {
                            FunctionMutator::from(&mut *function).replace_next_block(
                                previous_block_id,
                                block_id,
                                target_block,
                            );
                        }
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
            #[cfg(debug_assertions)]
            function.check_integrity().unwrap();
        }
        outcome
    }
}
