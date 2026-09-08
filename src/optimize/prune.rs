use crate::mir::{Function, FunctionMutator, Global, GlobalId, Instr, Local, Operand, Terminator};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use indexmap::IndexMap;
use slotmap::SlotMap;
use tracing::instrument;

/// Prune unnecessary structures within MIR functions
///
/// This includes:
///
/// - Removing `Nop` instructions produced by other optimization rounds
/// - Joining sequential basic blocks
/// - Removing unnecessary empty basic blocks
pub struct Prune;

impl Optimize for Prune {
    #[instrument(name = "prune", skip_all, fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(
        &self,
        function: &mut Function,
        _globals: &SlotMap<GlobalId, Global>,
        _ty_registry: &TyRegistry,
    ) -> Outcome {
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
                                    .filter_map(|instruction| {
                                        match instruction {
                                            Instr::Phi { target, sources } => {
                                                let source = sources[block_id];
                                                if function.locals[source]
                                                    .defined
                                                    .contains_key(previous_block_id)
                                                {
                                                    // the source is defined in the previous block so
                                                    // the phi source would be able to be trivially patched
                                                    None
                                                } else {
                                                    Some((*target, source))
                                                }
                                            }
                                            _ => None,
                                        }
                                    })
                                    .collect::<IndexMap<_, _>>()
                            } else {
                                IndexMap::new()
                            };
                        if !phi_sources.is_empty() {
                            for (index, (_, source)) in phi_sources.iter_mut().enumerate() {
                                let target = function.locals.insert(Local {
                                    ty: function.locals[*source].ty,
                                    is_return: false,
                                    defined: [(block_id, index)].into_iter().collect(),
                                    name: function.locals[*source].name.clone(),
                                });
                                function.blocks[block_id].instructions.push(Instr::Copy {
                                    target,
                                    source: Operand::Local(*source),
                                });
                                *source = target;
                            }
                            for instruction in function.blocks[target_block].instructions.iter_mut()
                            {
                                let Instr::Phi { target, sources } = instruction else {
                                    continue;
                                };
                                let Some(&new_source) = phi_sources.get(&*target) else {
                                    continue;
                                };
                                sources[block_id] = new_source;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Identifier, IntegerLiteral};
    use crate::mir::Condition;
    use crate::ty::Ty;
    use slotmap::{SlotMap, SparseSecondaryMap};
    use std::assert_matches;

    fn new_builder() -> crate::mir::FunctionBuilder {
        crate::mir::FunctionBuilder::new(Identifier::from("f"), Ty::U8)
    }

    fn run_prune(function: &mut Function) -> Outcome {
        Prune.optimize_function(function, &SlotMap::with_key(), &TyRegistry::default())
    }

    fn instr_count(function: &Function) -> usize {
        function.blocks.values().map(|b| b.instructions.len()).sum()
    }

    fn literal(target: crate::mir::LocalId, value: u32) -> Instr {
        Instr::Copy {
            target,
            source: Operand::Literal(IntegerLiteral::from(value)),
        }
    }

    // --- Nop removal + local re-indexing ---

    #[test]
    fn removes_nop_instructions() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        fb.push_instr(block, Instr::Nop);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();

        let outcome = run_prune(&mut function);

        assert_eq!(outcome, Outcome::Modified);
        assert_eq!(instr_count(&function), 1);
        assert!(!matches!(
            function.blocks[block].instructions[0],
            Instr::Nop
        ));
    }

    #[test]
    fn reindexes_local_definitions_after_removing_a_leading_nop() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        fb.push_instr(block, Instr::Nop);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();
        assert_eq!(
            function.locals[ret].defined[block], 1,
            "sanity: before prune, ret is defined at index 1"
        );

        run_prune(&mut function);

        assert_eq!(
            function.locals[ret].defined[block], 0,
            "removing the leading Nop must shift ret's recorded definition index down"
        );
        // check_integrity cross-checks target_local() against the recorded `defined` index, so
        // this call fails if the re-indexing above is wrong.
        function.check_integrity().unwrap();
    }

    #[test]
    fn leaves_function_with_no_nops_or_empty_blocks_unmodified() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();

        assert_eq!(run_prune(&mut function), Outcome::Unmodified);
    }

    // --- Joining a block into its sole unconditional predecessor ---

    #[test]
    fn joins_block_into_its_only_predecessor_when_predecessor_just_jumps_to_it() {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        fb.push_instr(
            b1,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(b1, Terminator::Jump { target_block: b2 });
        fb.set_terminator(b2, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        let mut function = fb.build();
        assert_eq!(function.blocks.len(), 2);

        let outcome = run_prune(&mut function);

        assert_eq!(outcome, Outcome::Modified);
        assert_eq!(
            function.blocks.len(),
            1,
            "b2 must be merged into b1 and removed"
        );
        function.check_integrity().unwrap();
    }

    // --- ReturnIf collapsing to Return when the else arm is an empty Return block ---

    #[test]
    fn collapses_return_if_into_return_when_else_block_is_an_empty_return() {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(
            b1,
            Instr::Copy {
                target: cond,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        fb.push_instr(
            b1,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(
            b1,
            Terminator::ReturnIf {
                source: cond,
                condition: Condition::NotZero,
                else_block: b2,
            },
        );
        fb.set_terminator(b2, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        let mut function = fb.build();

        let outcome = run_prune(&mut function);

        assert_eq!(outcome, Outcome::Modified);
        assert!(matches!(function.blocks[b1].terminator, Terminator::Return));
        function.check_integrity().unwrap();
    }

    #[test]
    fn empty_block_feeding_a_multi_predecessor_phi_is_patched_with_copies() {
        let mut fb = new_builder();
        let b1 = fb.new_block(); // branches to b2 (empty) or b4
        let b2 = fb.new_block(); // empty passthrough to b3
        let b3 = fb.new_block(); // phi merge point, then Return
        let b4 = fb.new_block(); // other arm, defines its own phi source

        let ret = fb.create_return_local(Ty::U8);
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(
            b1,
            Instr::Copy {
                target: cond,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        let x1 = fb.new_local(Ty::U8);
        fb.push_instr(
            b1,
            Instr::Copy {
                target: x1,
                source: Operand::Literal(IntegerLiteral::from(11)),
            },
        );
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b2,
                else_block: b4,
            },
        );

        // b2 is empty: it just forwards x1 (defined upstream in b1) on to b3's phi.
        fb.set_terminator(b2, Terminator::Jump { target_block: b3 });

        let x2 = fb.new_local(Ty::U8);
        fb.push_instr(
            b4,
            Instr::Copy {
                target: x2,
                source: Operand::Literal(IntegerLiteral::from(22)),
            },
        );
        fb.set_terminator(b4, Terminator::Jump { target_block: b3 });

        let phi_target = fb.new_local(Ty::U8);
        let mut sources = SparseSecondaryMap::new();
        sources.insert(b2, x1);
        sources.insert(b4, x2);
        fb.push_instr(
            b3,
            Instr::Phi {
                target: phi_target,
                sources,
            },
        );
        fb.push_instr(
            b3,
            Instr::Copy {
                target: ret,
                source: Operand::Local(phi_target),
            },
        );
        fb.set_terminator(b3, Terminator::Return);

        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b4);
        fb.seal_block(b3);
        let mut function = fb.build();
        function.check_integrity().unwrap();

        // run to a fixpoint the way `Optimizer` would
        while run_prune(&mut function) == Outcome::Modified {}

        assert!(!function.blocks.contains_key(b2), "b2 should be removed");
        assert_matches!(
            function.blocks[b1].terminator,
            Terminator::JumpIf { then_block, .. } if then_block == b3,
            "b1 should jumps to b3"
        );
        function.check_integrity().unwrap();
    }

    #[test]
    fn empty_block_feeding_two_phis_with_mixed_triviality_does_not_panic() {
        let mut fb = new_builder();
        let entry = fb.new_block(); // defines w, used only by the non-trivial phi
        let mid1 = fb.new_block(); // non-empty, one of p's two real predecessors
        let mid2 = fb.new_block(); // non-empty, the other of p's two real predecessors
        let p = fb.new_block(); // previous_block_id: defines p_local, used by the trivial phi
        let b = fb.new_block(); // block_id: empty passthrough to t
        let a = fb.new_block(); // t's other predecessor
        let t = fb.new_block(); // has two phis merging the b/a edges

        let ret = fb.create_return_local(Ty::U8);
        let w = fb.new_local(Ty::U8);
        fb.push_instr(entry, literal(w, 100));
        let branch_cond = fb.new_local(Ty::U1);
        fb.push_instr(entry, literal(branch_cond, 0));
        fb.set_terminator(
            entry,
            Terminator::JumpIf {
                source: branch_cond,
                condition: Condition::NotZero,
                then_block: mid1,
                else_block: mid2,
            },
        );

        // mid1/mid2 are non-empty (so Prune can't fold them away as trivial pass-throughs) and
        // give p two real, permanent predecessors, so p (and therefore p_local) can never be
        // merged elsewhere - unlike w's defining block, which must stay genuinely distinct from
        // p for this scenario to actually exercise the mixed-triviality path.
        let junk1 = fb.new_local(Ty::U8);
        fb.push_instr(mid1, literal(junk1, 5));
        fb.set_terminator(mid1, Terminator::Jump { target_block: p });
        let junk2 = fb.new_local(Ty::U8);
        fb.push_instr(mid2, literal(junk2, 6));
        fb.set_terminator(mid2, Terminator::Jump { target_block: p });

        let p_local = fb.new_local(Ty::U8);
        fb.push_instr(p, literal(p_local, 2));
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(p, literal(cond, 0));
        fb.set_terminator(
            p,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b,
                else_block: a,
            },
        );

        fb.set_terminator(b, Terminator::Jump { target_block: t });

        let y_from_a = fb.new_local(Ty::U8);
        fb.push_instr(a, literal(y_from_a, 50));
        let z_from_a = fb.new_local(Ty::U8);
        fb.push_instr(a, literal(z_from_a, 60));
        fb.set_terminator(a, Terminator::Jump { target_block: t });

        let y = fb.new_local(Ty::U8);
        let mut y_sources = SparseSecondaryMap::new();
        // trivial: p_local is already defined in p (previous_block_id for b's edge)
        y_sources.insert(b, p_local);
        y_sources.insert(a, y_from_a);
        fb.push_instr(
            t,
            Instr::Phi {
                target: y,
                sources: y_sources,
            },
        );
        let z = fb.new_local(Ty::U8);
        let mut z_sources = SparseSecondaryMap::new();
        // non-trivial: w is defined in entry, not in p
        z_sources.insert(b, w);
        z_sources.insert(a, z_from_a);
        fb.push_instr(
            t,
            Instr::Phi {
                target: z,
                sources: z_sources,
            },
        );
        fb.push_instr(
            t,
            Instr::Add {
                target: ret,
                left: y,
                right: Operand::Local(z),
            },
        );
        fb.set_terminator(t, Terminator::Return);

        fb.seal_block(entry);
        fb.seal_block(mid1);
        fb.seal_block(mid2);
        fb.seal_block(p);
        fb.seal_block(b);
        fb.seal_block(a);
        fb.seal_block(t);
        let mut function = fb.build();
        function.check_integrity().unwrap();

        run_prune(&mut function);
    }
}
