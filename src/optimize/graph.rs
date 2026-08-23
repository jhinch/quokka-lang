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
    ///  Figure 3: The Engineered Algorithm
    #[must_use]
    pub fn calculate_immediate_dominators(&self) -> IndexMap<Node, Node> {
        let reverse_postorder = {
            let mut postorder = self
                .postorder()
                .enumerate()
                .map(|(index, node)| (node, self.forward_edges.len() - index))
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
                            // u16 so that it panics in debug mode if reach u16::MAX
                            let mut count: u16 = 0;
                            while finger1 != finger2 {
                                while reverse_postorder[&finger1] > reverse_postorder[&finger2] {
                                    finger1 = immediate_dominators[&finger1];
                                    tracing::trace!(
                                        "Consider immediate_dominator for {node:?}. finger1={finger1:?}, finger2={finger2:?}"
                                    );
                                    count += 1;
                                }
                                while reverse_postorder[&finger2] > reverse_postorder[&finger1] {
                                    finger2 = immediate_dominators[&finger2];
                                    tracing::trace!(
                                        "Consider immediate_dominator for {node:?}. finger1={finger1:?}, finger2={finger2:?}"
                                    );
                                    count += 1;
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
    ///  Figure 5: The Dominance-Frontier Algorithm
    #[must_use]
    #[instrument(skip(self))]
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
