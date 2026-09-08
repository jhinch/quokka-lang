use crate::mir::{BlockId, Function, FunctionMutator, Global, GlobalId, Instr, Terminator};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use indexmap::{IndexMap, IndexSet};
use slotmap::SlotMap;
use std::collections::VecDeque;
use tracing::instrument;

/// Aggressive Dead Code Elimination (ADCE)
///
/// ADCE eliminates dead code within each function. See [`super::tree_shake`] for
/// elimination of dead functions and globals.
///
/// Algorithm pseudocode is
/// from ( "Engineering a Compiler", 2004)
/// Figure 10.3
/// by Keith D. Cooper & Linda Torczon
pub struct Adce;

impl Optimize for Adce {
    #[instrument(name = "adce", skip_all, fields(function = %function.name), ret(level = tracing::Level::DEBUG))]
    fn optimize_function(
        &self,
        function: &mut Function,
        _globals: &SlotMap<GlobalId, Global>,
        _ty_registry: &TyRegistry,
    ) -> Outcome {
        let graph = Graph::from(&*function).reverse();
        let (reverse_dominance_frontier, immediate_post_dominators) =
            graph.calculate_dominance_frontier();
        let marked = self.mark_pass(function, &reverse_dominance_frontier);
        self.sweep_pass(function, marked, &immediate_post_dominators)
    }
}

impl Adce {
    #[instrument(skip_all)]
    fn mark_pass(
        &self,
        function: &Function,
        reverse_dominance_frontier: &IndexMap<Node, IndexSet<Node>>,
    ) -> Marked {
        let mut alive_instructions = IndexSet::new();
        let mut required_branches = IndexSet::new();
        let mut required_blocks = IndexSet::new();
        let mut queue = VecDeque::new();
        for block_id in function.traverse_blocks() {
            let block = &function.blocks[block_id];
            for (index, instruction) in block.instructions.iter().enumerate() {
                if matches!(instruction, Instr::Load { .. } | Instr::Store { .. })
                    && alive_instructions.insert((block_id, index))
                {
                    queue.push_back(WorkItem::LiveInstr(block_id, index));
                }
            }
            if matches!(
                &block.terminator,
                Terminator::Return | Terminator::ReturnIf { .. }
            ) && required_blocks.insert(block_id)
            {
                queue.push_back(WorkItem::LiveBlock(block_id));
            }
        }

        while let Some(work) = queue.pop_front() {
            match work {
                WorkItem::LiveInstr(block_id, index) => {
                    if required_blocks.insert(block_id) {
                        queue.push_back(WorkItem::LiveBlock(block_id));
                    }
                    for source in function.blocks[block_id].instructions[index].source_locals() {
                        let (source_block_id, &index) =
                            function.locals[source].defined.iter().next().unwrap();
                        if alive_instructions.insert((source_block_id, index)) {
                            queue.push_back(WorkItem::LiveInstr(source_block_id, index));
                        }
                    }
                }
                WorkItem::LiveBlock(block_id) => {
                    match &function.blocks[block_id].terminator {
                        terminator @ Terminator::Return
                        | terminator @ Terminator::ReturnIf { .. } => {
                            if let Some(return_local) = function.return_local {
                                let index = function.locals[return_local].defined[block_id];
                                if alive_instructions.insert((block_id, index)) {
                                    queue.push_back(WorkItem::LiveInstr(block_id, index));
                                }
                            }
                            for source in terminator.source_locals() {
                                let (source_block_id, &index) =
                                    function.locals[source].defined.iter().next().unwrap();
                                if alive_instructions.insert((source_block_id, index)) {
                                    queue.push_back(WorkItem::LiveInstr(source_block_id, index));
                                }
                            }
                        }
                        _ => {}
                    }
                    for node in reverse_dominance_frontier
                        .get(&Node::from(block_id))
                        .into_iter()
                        .flatten()
                        .copied()
                    {
                        if let Node::Block(dominant_block_id) = node {
                            required_branches.insert(dominant_block_id);
                            for source in function.blocks[dominant_block_id]
                                .terminator
                                .source_locals()
                            {
                                let (source_block_id, &index) =
                                    function.locals[source].defined.iter().next().unwrap();
                                if alive_instructions.insert((source_block_id, index)) {
                                    queue.push_back(WorkItem::LiveInstr(source_block_id, index));
                                }
                            }
                            if required_blocks.insert(dominant_block_id) {
                                queue.push_back(WorkItem::LiveBlock(dominant_block_id));
                            }
                        }
                    }
                }
            }
        }
        Marked {
            alive_instructions,
            required_blocks,
            required_branches,
        }
    }

    #[instrument(skip_all, fields(function = %function.name))]
    fn sweep_pass(
        &self,
        function: &mut Function,
        marked: Marked,
        immediate_post_dominators: &IndexMap<Node, Node>,
    ) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for block_id in function.blocks.keys().collect::<Vec<_>>() {
            match &function.blocks[block_id].terminator {
                Terminator::CallIf {
                    return_block: then_block,
                    else_block,
                    ..
                }
                | Terminator::JumpIf {
                    then_block,
                    else_block,
                    ..
                } if !marked.required_branches.contains(&block_id) => {
                    if let Some(target_block) = nearest_required_post_dominator(
                        block_id,
                        immediate_post_dominators,
                        &marked.required_blocks,
                    ) {
                        let then_block = *then_block;
                        let else_block = *else_block;
                        tracing::trace!(
                            "Updated {:?} in {block_id:?} with to target_block={target_block:?}",
                            function.blocks[block_id].terminator
                        );
                        if then_block != target_block {
                            outcome = Outcome::Modified;
                            FunctionMutator::from(&mut *function).replace_next_block(
                                block_id,
                                then_block,
                                target_block,
                            );
                        }
                        if else_block != target_block {
                            outcome = Outcome::Modified;
                            FunctionMutator::from(&mut *function).replace_next_block(
                                block_id,
                                else_block,
                                target_block,
                            );
                        }
                    }
                }
                Terminator::ReturnIf { else_block, .. }
                    if !marked.required_branches.contains(&block_id) =>
                {
                    let else_block = *else_block;
                    tracing::trace!(
                        "Updated {:?} in {block_id:?} with return",
                        function.blocks[block_id].terminator
                    );
                    FunctionMutator::from(&mut *function).remove_next_block(block_id, else_block);
                }
                _ => {}
            }

            for (index, instrument) in function.blocks[block_id]
                .instructions
                .iter_mut()
                .enumerate()
                .rev()
            {
                if !marked.alive_instructions.contains(&(block_id, index)) {
                    let existing = std::mem::replace(instrument, Instr::Nop);
                    if !matches!(existing, Instr::Nop) {
                        outcome = Outcome::Modified;
                        tracing::debug!("Dropping dead {existing:?} from block {block_id:?}");
                    }
                    if let Some(target) = existing.target_local() {
                        function.locals.remove(target);
                    }
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

fn nearest_required_post_dominator(
    block: BlockId,
    immediate_post_dominators: &IndexMap<Node, Node>,
    required_blocks: &IndexSet<BlockId>,
) -> Option<BlockId> {
    let mut cur: Node = Node::from(block);
    loop {
        cur = *immediate_post_dominators.get(&cur)?;
        if let Node::Block(cur) = cur
            && required_blocks.contains(&cur)
        {
            return Some(cur);
        }
    }
}

struct Marked {
    alive_instructions: IndexSet<(BlockId, usize)>,
    required_branches: IndexSet<BlockId>,
    required_blocks: IndexSet<BlockId>,
}

enum WorkItem {
    LiveInstr(BlockId, usize),
    LiveBlock(BlockId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Identifier, IntegerLiteral, SpannedIdentifier};
    use crate::attr::GlobalDeclAttributes;
    use crate::mir::{Condition, FunctionBuilder, GlobalAttributes, LocalId, Operand};
    use crate::ty::Ty;
    use miette::SourceSpan;
    use slotmap::SparseSecondaryMap;

    fn new_builder() -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from("f"), Ty::U8)
    }

    fn run_adce(function: &mut Function, globals: &SlotMap<GlobalId, Global>) -> Outcome {
        Adce.optimize_function(function, globals, &TyRegistry::default())
    }

    fn dummy_global(ty: Ty) -> (SlotMap<GlobalId, Global>, GlobalId) {
        let mut globals = SlotMap::with_key();
        let id = globals.insert(Global {
            ty,
            name: SpannedIdentifier::new("G", SourceSpan::new(0.into(), 0)),
            attrs: GlobalAttributes::Ram(GlobalDeclAttributes::default()),
            value: Vec::new(),
        });
        (globals, id)
    }

    fn literal(target: LocalId, value: u32) -> Instr {
        Instr::Copy {
            target,
            source: Operand::Literal(IntegerLiteral::from(value)),
        }
    }

    // --- dead code sweep ---

    #[test]
    fn dead_instruction_becomes_nop_and_its_local_is_removed() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let one = fb.new_local(Ty::U8);
        fb.push_instr(block, literal(one, 1));
        let dead = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Add {
                target: dead,
                left: one,
                right: Operand::Local(one),
            },
        );
        fb.push_instr(block, literal(ret, 0));
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();
        let (globals, _) = dummy_global(Ty::U8);

        let outcome = run_adce(&mut function, &globals);

        assert_eq!(outcome, Outcome::Modified);
        assert!(matches!(function.blocks[block].instructions[1], Instr::Nop));
        assert!(
            !function.locals.contains_key(dead),
            "the dead instruction's target local must be removed"
        );
        function.check_integrity().unwrap();
    }

    #[test]
    fn load_is_kept_alive_even_when_its_result_is_unused() {
        let (globals, global_id) = dummy_global(Ty::U8);
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let unused = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Load {
                target: unused,
                source: global_id,
                projections: Vec::new(),
            },
        );
        fb.push_instr(block, literal(ret, 0));
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();

        let outcome = run_adce(&mut function, &globals);

        assert_eq!(
            outcome,
            Outcome::Unmodified,
            "Load is a root and must survive"
        );
        assert!(matches!(
            function.blocks[block].instructions[0],
            Instr::Load { .. }
        ));
    }

    #[test]
    fn store_is_kept_alive_even_though_it_has_no_target_local() {
        let (globals, global_id) = dummy_global(Ty::U8);
        let mut fb = new_builder();
        let block = fb.new_block();
        let value = fb.new_local(Ty::U8);
        fb.push_instr(block, literal(value, 7));
        fb.push_instr(
            block,
            Instr::Store {
                target: global_id,
                projections: Vec::new(),
                source: Operand::Local(value),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let mut function = fb.build();

        let outcome = run_adce(&mut function, &globals);

        assert_eq!(
            outcome,
            Outcome::Unmodified,
            "Store is a root and must survive"
        );
        assert!(matches!(
            function.blocks[block].instructions[1],
            Instr::Store { .. }
        ));
    }

    // --- branch liveness / redirection to the nearest required post-dominator ---

    #[test]
    fn branch_with_two_side_effect_free_arms_collapses_to_an_unconditional_jump() {
        let mut fb = new_builder();
        let b1 = fb.new_block(); // branch
        let b2 = fb.new_block(); // then: empty
        let b3 = fb.new_block(); // else: empty
        let b4 = fb.new_block(); // merge: the only required block
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
        fb.set_terminator(b2, Terminator::Jump { target_block: b4 });
        fb.set_terminator(b3, Terminator::Jump { target_block: b4 });
        fb.push_instr(b4, literal(ret, 5));
        fb.set_terminator(b4, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        fb.seal_block(b4);
        let mut function = fb.build();
        let (globals, _) = dummy_global(Ty::U8);

        let outcome = run_adce(&mut function, &globals);

        assert_eq!(outcome, Outcome::Modified);
        assert!(
            matches!(
                function.blocks[b1].terminator,
                Terminator::Jump { target_block } if target_block == b4
            ),
            "the branch determines nothing observable, so it must collapse to a plain jump \
             straight to the merge block"
        );
        assert!(!function.blocks.contains_key(b2));
        assert!(!function.blocks.contains_key(b3));
        function.check_integrity().unwrap();
    }

    #[test]
    fn branch_whose_arms_produce_different_live_results_is_preserved() {
        let mut fb = new_builder();
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
        fb.push_instr(b2, literal(x2, 1));
        fb.set_terminator(b2, Terminator::Jump { target_block: b4 });
        let x3 = fb.new_local(Ty::U8);
        fb.push_instr(b3, literal(x3, 2));
        fb.set_terminator(b3, Terminator::Jump { target_block: b4 });
        let mut sources = SparseSecondaryMap::new();
        sources.insert(b2, x2);
        sources.insert(b3, x3);
        let phi = fb.new_local(Ty::U8);
        fb.push_instr(
            b4,
            Instr::Phi {
                target: phi,
                sources,
            },
        );
        fb.push_instr(
            b4,
            Instr::Copy {
                target: ret,
                source: Operand::Local(phi),
            },
        );
        fb.set_terminator(b4, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        fb.seal_block(b4);
        let mut function = fb.build();
        let (globals, _) = dummy_global(Ty::U8);

        let outcome = run_adce(&mut function, &globals);

        assert_eq!(
            outcome,
            Outcome::Unmodified,
            "the branch controls the returned value, so nothing should be pruned"
        );
        assert!(matches!(
            function.blocks[b1].terminator,
            Terminator::JumpIf { .. }
        ));
    }
}
