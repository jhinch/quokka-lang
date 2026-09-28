use crate::mir::{BlockId, Function, Terminator};
use indexmap::map::Entry;
use indexmap::{IndexMap, IndexSet};
use std::collections::VecDeque;
use std::fmt;
use tracing::instrument;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    Block(BlockId),
    Exit,
}

impl From<BlockId> for Node {
    fn from(block: BlockId) -> Self {
        Node::Block(block)
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Node::Block(block) => write!(f, "{block}"),
            Node::Exit => write!(f, "<exit>"),
        }
    }
}

/// A separated Control flow graph built from a MIR CFG.
/// It is built separately to allow for it two additional
/// pieces of functionally:
///
/// 1. A synthetic [`Node::Exit`] is added for all end blocks and
///    blocks which infinite loop.
/// 2. [`Graph::reverse`] can be used to switch the direction of edges,
///    allowing for immediate post-dominators (i.e. reverse dominance) and
///    reverse dominance frontier to be easily be calculated with the same
///    implementation
pub struct Graph {
    source: Node,
    sink: Node,
    // use IndexMap and IndexSet here not because the order is relevant to the graph
    // but in order to create repeatability via a stable order
    forward_edges: IndexMap<Node, IndexSet<Node>>,
    backward_edges: IndexMap<Node, IndexSet<Node>>,
}

impl fmt::Debug for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct DebugEdgesMap<'a>(&'a IndexMap<Node, IndexSet<Node>>);
        impl fmt::Debug for DebugEdgesMap<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut debug_map = f.debug_map();
                for (node, edges) in self.0 {
                    debug_map.entry(node, &DebugEdgesSet(edges));
                }
                debug_map.finish()
            }
        }
        struct DebugEdgesSet<'a>(&'a IndexSet<Node>);
        impl fmt::Debug for DebugEdgesSet<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut debug_set = f.debug_set();
                for node in self.0 {
                    debug_set.entry(node);
                }
                debug_set.finish()
            }
        }
        f.debug_struct("Graph")
            .field("source", &self.source)
            .field("sink", &self.sink)
            .field("forward_edges", &DebugEdgesMap(&self.forward_edges))
            .field("backward_edges", &DebugEdgesMap(&self.backward_edges))
            .finish()
    }
}

impl Graph {
    /// reverse the direction of the graph edges
    #[must_use]
    pub fn reverse(self) -> Self {
        Graph {
            source: self.sink,
            sink: self.source,
            forward_edges: self.backward_edges,
            backward_edges: self.forward_edges,
        }
    }

    /// Calculate the immediate dominators for each node.
    ///
    /// Node A strictly dominates B if every path from the `source` node to B must go through A and
    /// A != B.
    ///
    /// The immediate dominator D of node N is the unique node that strictly dominates N but
    /// does not strictly dominate any other node that strictly dominates N.
    ///
    /// Every node in the graph will have an immediate dominator except the `source`.
    ///
    /// Algorithm uses the Cooper-Harvey-Kennedy iterative dominance algorithm
    /// from ("A Simple, Fast Dominance Algorithm", 2001).
    /// Figure 3: The Engineered Algorithm
    #[must_use]
    pub fn calculate_immediate_dominators(&self) -> IndexMap<Node, Node> {
        let reverse_postorder = {
            let mut postorder = self
                .postorder()
                .enumerate()
                .map(|(index, node)| (node, index))
                .collect::<IndexMap<_, _>>();
            postorder.reverse();
            postorder
        };
        tracing::trace!("Calculated reverse postorder as {reverse_postorder:?}");

        let mut immediate_dominators = IndexMap::new();
        immediate_dominators.insert(self.source, self.source);

        loop {
            let mut changed = false;
            for &node in reverse_postorder.keys() {
                if node == self.source {
                    continue;
                }
                let mut new_immediate_dominator = None;
                for candidate in self.backward_edges[&node]
                    .iter()
                    .copied()
                    .filter(|n| immediate_dominators.contains_key(n))
                {
                    tracing::trace!(
                        "Consider immediate_dominator for {node:?}. candidate={candidate:?}, existing={new_immediate_dominator:?}"
                    );
                    new_immediate_dominator = Some(match new_immediate_dominator {
                        None => candidate,
                        Some(existing) => {
                            // intersect `candidate` and `existing`
                            // using the reverse postorder and immediate_dominators
                            let mut finger1 = candidate;
                            let mut finger2 = existing;
                            while finger1 != finger2 {
                                while reverse_postorder[&finger1] < reverse_postorder[&finger2] {
                                    finger1 = immediate_dominators[&finger1];
                                    tracing::trace!(
                                        "Consider immediate_dominator for {node:?}. finger1={finger1:?}, finger2={finger2:?}"
                                    );
                                }
                                while reverse_postorder[&finger2] < reverse_postorder[&finger1] {
                                    finger2 = immediate_dominators[&finger2];
                                    tracing::trace!(
                                        "Consider immediate_dominator for {node:?}. finger1={finger1:?}, finger2={finger2:?}"
                                    );
                                }
                            }
                            finger1
                        }
                    });
                }
                if let Some(new_immediate_dominator) = new_immediate_dominator {
                    match immediate_dominators.entry(node) {
                        Entry::Occupied(mut entry) => {
                            if *entry.get() != new_immediate_dominator {
                                entry.insert(new_immediate_dominator);
                                changed = true;
                            }
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(new_immediate_dominator);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        immediate_dominators.swap_remove_entry(&self.source);
        immediate_dominators
    }

    /// Visit each node after all the nodes it is connected to by a forward edge have been visited
    pub fn postorder(&self) -> impl Iterator<Item = Node> + '_ {
        let mut visited: IndexSet<Node> = IndexSet::new();
        let mut stack: Vec<(Node, indexmap::set::Iter<'_, Node>)> =
            vec![(self.source, self.forward_edges[&self.source].iter())];
        visited.insert(self.source);
        std::iter::from_fn(move || {
            while let Some(&mut (node, ref mut edges)) = stack.last_mut() {
                match edges.next() {
                    Some(&forward_node) => {
                        if visited.insert(forward_node) {
                            stack.push((forward_node, self.forward_edges[&forward_node].iter()));
                        }
                    }
                    None => {
                        stack.pop();
                        return Some(node);
                    }
                }
            }
            None
        })
    }

    /// Calculate the dominance frontier.
    /// The dominance frontier node A is the set of all nodes, B, such that A dominates a predecessor of B but does not strictly
    /// dominate B.
    ///
    /// The immediate dominators calculated from [`Graph::calculate_immediate_dominators`]
    /// are also returned.
    ///
    /// Algorithm uses the Cooper-Harvey-Kennedy dominance frontier algorithm
    /// from ("A Simple, Fast Dominance Algorithm", 2001).
    /// Figure 5: The Dominance-Frontier Algorithm
    #[must_use]
    #[instrument(skip_all)]
    pub fn calculate_dominance_frontier(
        &self,
    ) -> (IndexMap<Node, IndexSet<Node>>, IndexMap<Node, Node>) {
        let immediate_dominators = self.calculate_immediate_dominators();
        let mut dominance_frontier = IndexMap::<Node, IndexSet<Node>>::new();
        for (&node, edges) in self
            .backward_edges
            .iter()
            .filter(|(_, edges)| edges.len() >= 2)
        {
            let immediate_dominator = immediate_dominators[&node];
            for &backward_node in edges {
                let mut runner = backward_node;
                while runner != immediate_dominator {
                    dominance_frontier.entry(runner).or_default().insert(node);
                    runner = immediate_dominators[&runner];
                }
            }
        }
        (dominance_frontier, immediate_dominators)
    }
}

impl From<&Function> for Graph {
    fn from(function: &Function) -> Self {
        let source = Node::from(function.start_block);
        let sink = Node::Exit;
        let mut backward_edges = IndexMap::<Node, IndexSet<Node>>::new();
        let mut forward_edges = function
            .traverse_blocks()
            .map(|block| {
                let mut edges = function.blocks[block]
                    .next_blocks()
                    .map(Node::from)
                    .collect::<IndexSet<_>>();
                if matches!(
                    function.blocks[block].terminator,
                    Terminator::Return | Terminator::ReturnIf { .. }
                ) {
                    edges.insert(sink);
                }
                // ensure that each node has a backward edges entry
                backward_edges.entry(Node::from(block)).or_default();
                for &node in &edges {
                    backward_edges
                        .entry(node)
                        .or_default()
                        .insert(Node::from(block));
                }
                (Node::from(block), edges)
            })
            .collect::<IndexMap<_, _>>();
        forward_edges.entry(sink).or_default();
        backward_edges.entry(sink).or_default();
        let can_reach_exit = reachable(sink, &backward_edges);
        for (&node, edges) in forward_edges.iter_mut() {
            if !can_reach_exit.contains(&node) {
                // give all infinite loops nodes an edge to the exit node
                edges.insert(sink);
                backward_edges.entry(sink).or_default().insert(node);
            }
        }
        let graph = Graph {
            source,
            forward_edges,
            backward_edges,
            sink,
        };
        tracing::trace!("Graph for {} calculated {graph:?}", function.name);
        graph
    }
}

fn reachable(start: Node, forward_edges: &IndexMap<Node, IndexSet<Node>>) -> IndexSet<Node> {
    let mut seen = IndexSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start);
    queue.push_back(start);
    while let Some(u) = queue.pop_front() {
        for &v in &forward_edges[&u] {
            if seen.insert(v) {
                queue.push_back(v);
            }
        }
    }
    seen
}

/// Test-only helpers for building small [`Function`] control flow graphs of a given shape,
/// without needing real Quokka source or the SSA-var machinery in [`FunctionBuilder`].
///
/// These build purely structural CFGs (a single `u1` local is reused as the condition for every
/// branch): the graph algorithms under test only care about block topology, not instruction
/// semantics.
#[cfg(test)]
pub(crate) mod graph_test_support {
    use crate::ast::{Identifier, IntegerLiteral};
    use crate::mir::{BlockId, Function, FunctionBuilder, Instr, Operand, Terminator};
    use crate::ty::Ty;

    pub fn new_builder() -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from("f"), Ty::UNIT)
    }

    /// Pushes a fresh `u1` local into `block`, initialized to a literal, for use as a branch
    /// condition. The actual value is irrelevant to graph-shape tests.
    pub fn cond_local(builder: &mut FunctionBuilder, block: BlockId) -> crate::mir::LocalId {
        let local = builder.new_local(Ty::U1);
        builder.push_instr(
            block,
            Instr::Copy {
                target: local,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        local
    }

    pub fn seal_all(builder: &mut FunctionBuilder, blocks: &[BlockId]) {
        for &block in blocks {
            builder.seal_block(block);
        }
    }

    /// `bb1 -> bb2 -> bb3 -> Return`
    pub fn linear_chain() -> (Function, [BlockId; 3]) {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        fb.set_terminator(b1, Terminator::Jump { target_block: b2 });
        fb.set_terminator(b2, Terminator::Jump { target_block: b3 });
        fb.set_terminator(b3, Terminator::Return);
        seal_all(&mut fb, &[b1, b2, b3]);
        (fb.build(), [b1, b2, b3])
    }

    /// ```text
    ///      bb1
    ///     /    \
    ///   bb2    bb3
    ///     \    /
    ///      bb4
    ///       |
    ///     Return
    /// ```
    pub fn diamond() -> (Function, [BlockId; 4]) {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let b4 = fb.new_block();
        let cond = cond_local(&mut fb, b1);
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: crate::mir::Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        fb.set_terminator(b2, Terminator::Jump { target_block: b4 });
        fb.set_terminator(b3, Terminator::Jump { target_block: b4 });
        fb.set_terminator(b4, Terminator::Return);
        seal_all(&mut fb, &[b1, b2, b3, b4]);
        (fb.build(), [b1, b2, b3, b4])
    }

    /// ```text
    ///  bb1
    ///   |
    ///  bb2 <--+
    ///  / \    |
    /// bb4 bb3-+   (bb3 loops back to bb2, bb4 exits)
    ///  |
    /// Return
    /// ```
    pub fn loop_with_exit() -> (Function, [BlockId; 4]) {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let b4 = fb.new_block();
        fb.set_terminator(b1, Terminator::Jump { target_block: b2 });
        let cond = cond_local(&mut fb, b2);
        fb.set_terminator(
            b2,
            Terminator::JumpIf {
                source: cond,
                condition: crate::mir::Condition::NotZero,
                then_block: b3,
                else_block: b4,
            },
        );
        fb.set_terminator(b3, Terminator::Jump { target_block: b2 });
        fb.set_terminator(b4, Terminator::Return);
        // b2 has two predecessors (b1 and the b3 back edge) so it cannot be sealed until both
        // are known; b3 depends on b2 being built first for its terminator to reference it.
        seal_all(&mut fb, &[b1, b3, b4, b2]);
        (fb.build(), [b1, b2, b3, b4])
    }

    /// `bb1 <-> bb2`, neither block can reach a `Return`.
    pub fn infinite_loop() -> (Function, [BlockId; 2]) {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        fb.set_terminator(b1, Terminator::Jump { target_block: b2 });
        fb.set_terminator(b2, Terminator::Jump { target_block: b1 });
        seal_all(&mut fb, &[b1, b2]);
        // FunctionBuilder::build() calls remove_dangling(), which is fine here: both blocks
        // are reachable from start_block (b1), just none of them reach a return.
        (fb.build(), [b1, b2])
    }

    /// `bb1` branches straight to two separate `Return` blocks.
    pub fn two_returns() -> (Function, [BlockId; 3]) {
        let mut fb = new_builder();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let cond = cond_local(&mut fb, b1);
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: crate::mir::Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        fb.set_terminator(b2, Terminator::Return);
        fb.set_terminator(b3, Terminator::Return);
        seal_all(&mut fb, &[b1, b2, b3]);
        (fb.build(), [b1, b2, b3])
    }
}

#[cfg(test)]
mod tests {
    use super::graph_test_support::*;
    use super::*;

    // --- postorder ---

    #[test]
    fn postorder_visits_every_reachable_node_exactly_once() {
        let (function, [b1, b2, b3, b4]) = diamond();
        let graph = Graph::from(&function);
        let order = graph.postorder().collect::<Vec<_>>();
        let mut sorted = order.clone();
        sorted.sort();
        let mut expected = vec![
            Node::Block(b1),
            Node::Block(b2),
            Node::Block(b3),
            Node::Block(b4),
            Node::Exit,
        ];
        expected.sort();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn postorder_visits_children_before_their_only_parent_in_a_chain() {
        let (function, [b1, b2, b3]) = linear_chain();
        let graph = Graph::from(&function);
        let order = graph.postorder().collect::<Vec<_>>();
        let position = |node: Node| order.iter().position(|&n| n == node).unwrap();
        // in a linear chain each node has exactly one successor, so postorder must strictly
        // descend from the sink back to the source
        assert!(position(Node::Exit) < position(Node::Block(b3)));
        assert!(position(Node::Block(b3)) < position(Node::Block(b2)));
        assert!(position(Node::Block(b2)) < position(Node::Block(b1)));
    }

    #[test]
    fn postorder_handles_cycles_without_looping_forever() {
        let (function, [b1, b2, b3, b4]) = loop_with_exit();
        let graph = Graph::from(&function);
        let order = graph.postorder().collect::<Vec<_>>();
        let mut sorted = order.clone();
        sorted.sort();
        let mut expected = vec![
            Node::Block(b1),
            Node::Block(b2),
            Node::Block(b3),
            Node::Block(b4),
            Node::Exit,
        ];
        expected.sort();
        assert_eq!(
            sorted, expected,
            "every node visited exactly once despite the back edge"
        );
    }

    // --- synthetic Exit edges for non-terminating control flow ---

    #[test]
    fn infinite_loop_blocks_get_synthetic_exit_edges() {
        let (function, [b1, b2]) = infinite_loop();
        let graph = Graph::from(&function);
        // both blocks are unable to reach a real Return, so Graph::from must give both of them
        // an edge to the synthetic Exit node, or dominance over Exit could not be computed
        let dominators = graph.calculate_immediate_dominators();
        assert_eq!(dominators[&Node::Exit], Node::Block(b1));
        assert_eq!(dominators[&Node::Block(b2)], Node::Block(b1));
    }

    // --- calculate_immediate_dominators (forward dominance) ---

    #[test]
    fn linear_chain_dominators_form_a_straight_line() {
        let (function, [b1, b2, b3]) = linear_chain();
        let graph = Graph::from(&function);
        let dominators = graph.calculate_immediate_dominators();
        assert_eq!(dominators[&Node::Block(b2)], Node::Block(b1));
        assert_eq!(dominators[&Node::Block(b3)], Node::Block(b2));
        assert_eq!(dominators[&Node::Exit], Node::Block(b3));
        // the source has no immediate dominator
        assert!(!dominators.contains_key(&Node::Block(b1)));
    }

    #[test]
    fn diamond_merge_block_is_dominated_by_the_branch_block() {
        let (function, [b1, b2, b3, b4]) = diamond();
        let graph = Graph::from(&function);
        let dominators = graph.calculate_immediate_dominators();
        assert_eq!(dominators[&Node::Block(b2)], Node::Block(b1));
        assert_eq!(dominators[&Node::Block(b3)], Node::Block(b1));
        // b4 is reachable via either arm, so its immediate dominator is the branch block
        // itself, not either arm
        assert_eq!(dominators[&Node::Block(b4)], Node::Block(b1));
    }

    #[test]
    fn loop_body_dominators_follow_the_single_entry_edge() {
        let (function, [b1, b2, b3, b4]) = loop_with_exit();
        let graph = Graph::from(&function);
        let dominators = graph.calculate_immediate_dominators();
        assert_eq!(dominators[&Node::Block(b2)], Node::Block(b1));
        assert_eq!(dominators[&Node::Block(b3)], Node::Block(b2));
        assert_eq!(dominators[&Node::Block(b4)], Node::Block(b2));
    }

    #[test]
    fn two_return_blocks_share_the_branch_block_as_exits_dominator() {
        let (function, [b1, _b2, _b3]) = two_returns();
        let graph = Graph::from(&function);
        let dominators = graph.calculate_immediate_dominators();
        assert_eq!(dominators[&Node::Exit], Node::Block(b1));
    }

    // --- calculate_dominance_frontier ---

    #[test]
    fn diamond_arms_have_the_merge_block_in_their_dominance_frontier() {
        let (function, [b1, b2, b3, b4]) = diamond();
        let graph = Graph::from(&function);
        let (frontier, _idom) = graph.calculate_dominance_frontier();
        assert_eq!(
            frontier.get(&Node::Block(b2)).cloned().unwrap_or_default(),
            IndexSet::from([Node::Block(b4)])
        );
        assert_eq!(
            frontier.get(&Node::Block(b3)).cloned().unwrap_or_default(),
            IndexSet::from([Node::Block(b4)])
        );
        // b1 dominates b4 outright, so b4 is not in b1's frontier
        assert!(
            frontier
                .get(&Node::Block(b1))
                .is_none_or(|set| !set.contains(&Node::Block(b4)))
        );
    }

    #[test]
    fn loop_body_has_the_header_in_its_dominance_frontier() {
        let (function, [_b1, b2, b3, _b4]) = loop_with_exit();
        let graph = Graph::from(&function);
        let (frontier, _idom) = graph.calculate_dominance_frontier();
        // b3's back edge target (the loop header b2) is not strictly dominated by b3, but b3
        // dominates a predecessor of b2 (itself), so b2 is in b3's frontier
        assert_eq!(
            frontier.get(&Node::Block(b3)).cloned().unwrap_or_default(),
            IndexSet::from([Node::Block(b2)])
        );
    }

    // --- reverse() (used to compute post-dominance) ---

    #[test]
    fn reverse_swaps_source_and_sink() {
        let (function, _ids) = diamond();
        let graph = Graph::from(&function);
        let reversed = graph.reverse();
        // reversing a Graph whose sink is always Node::Exit means the reversed graph's source
        // is Exit
        let (frontier, _idom) = reversed.calculate_dominance_frontier();
        let _ = frontier; // just confirm this doesn't panic; source/sink are private
        let doubly_reversed = reversed.reverse();
        let original_frontier = Graph::from(&function).calculate_dominance_frontier().0;
        let doubly_reversed_frontier = doubly_reversed.calculate_dominance_frontier().0;
        for (node, set) in &original_frontier {
            assert_eq!(doubly_reversed_frontier.get(node), Some(set));
        }
    }

    #[test]
    fn reverse_computes_post_dominance_for_a_diamond() {
        let (function, [b1, b2, b3, b4]) = diamond();
        let post_dom_graph = Graph::from(&function).reverse();
        let post_dominators = post_dom_graph.calculate_immediate_dominators();
        // every path out of b1 passes through b4 before reaching Exit, even though b1 has two
        // immediate successors
        assert_eq!(post_dominators[&Node::Block(b1)], Node::Block(b4));
        assert_eq!(post_dominators[&Node::Block(b2)], Node::Block(b4));
        assert_eq!(post_dominators[&Node::Block(b3)], Node::Block(b4));
    }

    #[test]
    fn reverse_computes_post_dominance_for_a_loop() {
        let (function, [b1, b2, b3, b4]) = loop_with_exit();
        let post_dom_graph = Graph::from(&function).reverse();
        let post_dominators = post_dom_graph.calculate_immediate_dominators();
        // every path from b1 must pass through the loop header b2 before reaching the exit
        assert_eq!(post_dominators[&Node::Block(b1)], Node::Block(b2));
        // b3 always jumps straight back to b2
        assert_eq!(post_dominators[&Node::Block(b3)], Node::Block(b2));
        // b4 is the only way out, so it post-dominates b2 itself
        assert_eq!(post_dominators[&Node::Block(b2)], Node::Block(b4));
    }

    #[test]
    fn node_debug_formats_block_and_exit() {
        let (_function, [b1, ..]) = diamond();
        assert_eq!(format!("{:?}", Node::Block(b1)), format!("{b1}"));
        assert_eq!(format!("{:?}", Node::Exit), "<exit>");
    }
}
