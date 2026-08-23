use crate::mir::{BlockId, Function, Instr, Mir, Terminator};
use crate::optimize::graph::{Graph, Node};
use crate::optimize::{Optimize, Outcome};
use crate::ty::TyRegistry;
use indexmap::{IndexMap, IndexSet};
use std::collections::VecDeque;
use tracing::instrument;

/// Aggressive Dead Code Elimination (ADCE)
///
/// Algorithm is an implementation of
/// "Engineering a Compiler" by Keith D. Cooper & Linda Torczon 2004
/// Figure 10.3
pub struct Adce;

impl Optimize for Adce {
    #[instrument(skip(self, mir, _ty_registry))]
    fn optimize(&self, mir: &mut Mir, _ty_registry: &TyRegistry) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for function in mir.functions.values_mut() {
            outcome |= self.optimize_function(function);
        }
        outcome
    }
}

impl Adce {
    #[instrument(skip(self, function), fields(function = %function.name))]
    fn optimize_function(&self, function: &mut Function) -> Outcome {
        let graph = Graph::from(&*function).reverse();
        let (reverse_dominance_frontier, immediate_post_dominators) =
            graph.calculate_dominance_frontier();
        let marked = self.mark_pass(function, &reverse_dominance_frontier);
        self.sweep_pass(function, marked, &immediate_post_dominators)
    }

    #[instrument(skip(self, function, reverse_dominance_frontier))]
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

    #[instrument(skip(self, function, marked, immediate_post_dominators), fields(function = %function.name))]
    fn sweep_pass(
        &self,
        function: &mut Function,
        marked: Marked,
        immediate_post_dominators: &IndexMap<Node, Node>,
    ) -> Outcome {
        let mut outcome = Outcome::Unmodified;
        for block_id in function.blocks.keys().collect::<Vec<_>>() {
            let updated_terminator = match std::mem::take(&mut function.blocks[block_id].terminator)
            {
                Terminator::CallIf {
                    source,
                    condition,
                    callee,
                    return_block,
                    else_block,
                } if !marked.required_branches.contains(&block_id) => {
                    if let Some(return_block) = nearest_required_post_dominator(
                        block_id,
                        immediate_post_dominators,
                        &marked.required_branches,
                    ) {
                        outcome = Outcome::Modified;
                        Terminator::Call {
                            callee,
                            return_block,
                        }
                    } else {
                        Terminator::CallIf {
                            source,
                            condition,
                            callee,
                            return_block,
                            else_block,
                        }
                    }
                }
                Terminator::JumpIf {
                    source,
                    condition,
                    then_block,
                    else_block,
                } if !marked.required_branches.contains(&block_id) => {
                    if let Some(target_block) = nearest_required_post_dominator(
                        block_id,
                        immediate_post_dominators,
                        &marked.required_blocks,
                    ) {
                        outcome = Outcome::Modified;
                        Terminator::Jump { target_block }
                    } else {
                        Terminator::JumpIf {
                            source,
                            condition,
                            then_block,
                            else_block,
                        }
                    }
                }
                Terminator::ReturnIf { .. } if !marked.required_branches.contains(&block_id) => {
                    outcome = Outcome::Modified;
                    Terminator::Return
                }
                terminator => terminator,
            };
            function.blocks[block_id].terminator = updated_terminator;

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
