use crate::lir::{BlockId, CodeSectionData, Instr, VirtualReg};
use crate::reg::{Color, ColorSet};
use indexmap::IndexSet;
use slotmap::SecondaryMap;
use std::collections::HashMap;
use tracing::instrument;

#[derive(Debug, Default)]
pub struct InterferenceGraph {
    edges: IndexSet<(VirtualReg, VirtualReg)>,
    node_to_edges: HashMap<VirtualReg, IndexSet<VirtualReg>>,
    node_to_data: HashMap<VirtualReg, NodeData>,
}

#[derive(Debug, Default)]
pub struct NodeData {
    pressure: usize,
    moves: IndexSet<(BlockId, usize)>,
    precolored: Option<Color>,
    permitted_colors: Option<ColorSet>,
}

impl InterferenceGraph {
    fn set_precolored(&mut self, a: VirtualReg, color: Color) {
        let data = self.node_to_data.entry(a).or_default();
        assert!(
            data.precolored.is_none(),
            "{a:?} is already precolored {:?}",
            data.precolored
        );
        assert!(
            data.permitted_colors
                .as_ref()
                .map(|set| set.contains(color))
                .unwrap_or(true),
            "cannot precolor {a:?} to {color:?} as it is only permitted to be {:?}",
            data.permitted_colors
        );
        data.precolored = Some(color);
        data.pressure = usize::MAX;
        data.permitted_colors = None;
    }

    pub fn constrain_colors_to(&mut self, a: VirtualReg, set: ColorSet) {
        let data = self.node_to_data.entry(a).or_default();
        assert!(
            data.precolored
                .map(|color| set.contains(color))
                .unwrap_or(true),
            "cannot constrain {a:?} to {set:?} as it is already precolored as {:?}",
            data.precolored
        );
    }

    fn add_edge(&mut self, a: VirtualReg, b: VirtualReg) {
        assert_ne!(a, b, "self edges are not permitted");
        if !self.edges.insert((a, b)) || !self.edges.insert((b, a)) {
            return;
        }

        self.add_pressure_from(a, b);
        self.add_pressure_from(b, a);
    }

    pub fn retarget_edge(&mut self, reg: VirtualReg, from: VirtualReg, to: VirtualReg) -> usize {
        let block_from = self.blocked_colors_count(reg, from);
        let is_new = reg != to && !self.edges.contains(&(reg, to));
        let block_to = if is_new {
            self.blocked_colors_count(reg, to)
        } else {
            0
        };

        self.edges.insert((reg, to));
        self.edges.insert((to, reg));
        if is_new && !self.is_precolored(reg) {
            self.node_to_edges.entry(reg).or_default().insert(to);
        }
        if is_new && !self.is_precolored(to) {
            self.node_to_edges.entry(to).or_default().insert(reg);
            let delta = self.blocked_colors_count(to, reg);
            let data = self.node_to_data.entry(to).or_default();
            data.pressure = data.pressure.saturating_add(delta);
        }
        block_from.checked_sub(block_to).unwrap()
    }

    fn add_pressure_from(&mut self, a: VirtualReg, b: VirtualReg) {
        if !self.is_precolored(a) && self.node_to_edges.entry(a).or_default().insert(b) {
            let delta = self.blocked_colors_count(a, b);
            let data = self.node_to_data.entry(a).or_default();
            data.pressure = data.pressure.saturating_add(delta);
        }
    }

    pub fn set_pressure(&mut self, reg: VirtualReg, value: usize) {
        self.node_to_data.entry(reg).or_default().pressure = value;
    }

    #[must_use]
    pub fn is_precolored(&self, a: VirtualReg) -> bool {
        self.node_to_data
            .get(&a)
            .map(|data| data.precolored.is_some())
            .unwrap_or(false)
    }

    #[must_use]
    pub fn precolor(&self, a: VirtualReg) -> Option<Color> {
        self.node_to_data.get(&a).and_then(|data| data.precolored)
    }

    /// the number of colors of node `a` which are blocked by colors in node `b`
    #[must_use]
    pub fn blocked_colors_count(&self, a: VirtualReg, b: VirtualReg) -> usize {
        self.permitted_colors(b)
            .into_iter()
            .map(|c1| {
                self.permitted_colors(a)
                    .into_iter()
                    .filter(|&c2| c1.conflicts(c2))
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    /// iterate over the permitted colors for the given virtual register
    #[must_use]
    pub fn permitted_colors(&self, a: VirtualReg) -> ColorSet {
        if let Some(data) = self.node_to_data.get(&a) {
            if let Some(color) = data.precolored {
                return ColorSet::just(color);
            }
            if let Some(permitted_colors) = &data.permitted_colors {
                return permitted_colors.clone();
            }
        }
        match a {
            VirtualReg::R8(_) => Color::r8(),
            VirtualReg::R16(_) => Color::r16(),
            VirtualReg::Flag(_) => Color::flags(),
        }
    }

    #[must_use]
    pub fn capacity_of(&self, a: VirtualReg) -> usize {
        if let Some(data) = self.node_to_data.get(&a) {
            if data.precolored.is_some() {
                return 1;
            }
            if let Some(permitted_colors) = &data.permitted_colors {
                return permitted_colors.len();
            }
        }
        match a {
            VirtualReg::R8(_) => Color::r8().len(),
            VirtualReg::R16(_) => Color::r16().len(),
            VirtualReg::Flag(_) => Color::flags().len(),
        }
    }

    #[must_use]
    pub fn pressure_of(&self, a: VirtualReg) -> usize {
        self.node_to_data
            .get(&a)
            .map(|data| data.pressure)
            .unwrap_or(0)
    }

    pub fn moves(&self, a: VirtualReg) -> impl Iterator<Item = (BlockId, usize)> {
        self.node_to_data
            .get(&a)
            .into_iter()
            .flat_map(|data| data.moves.iter().copied())
    }

    pub fn add_moves(&mut self, a: VirtualReg, moves: IndexSet<(BlockId, usize)>) {
        self.node_to_data.entry(a).or_default().moves.extend(moves);
    }

    pub fn get_edges(&self, a: VirtualReg) -> impl Iterator<Item = VirtualReg> {
        self.node_to_edges.get(&a).into_iter().flatten().copied()
    }

    #[must_use]
    pub fn has_edge(&self, a: VirtualReg, b: VirtualReg) -> bool {
        self.edges.contains(&(a, b))
    }
}

/// Returns the calculated interference graph + any moves detected
#[must_use]
#[instrument(skip_all)]
pub fn build_interference_graph(
    code: &CodeSectionData,
    BlockLiveness { live_out, .. }: &BlockLiveness,
) -> (InterferenceGraph, IndexSet<(BlockId, usize)>) {
    let mut graph = InterferenceGraph::default();
    for block in code.blocks.values() {
        for instruction in &block.instructions {
            restrict_from_instruction_targets(code, instruction, &mut graph);
        }
    }

    let mut moves = IndexSet::new();
    for (block_id, block) in code.blocks.iter() {
        let mut live: IndexSet<VirtualReg> = live_out[block_id].clone();
        for (index, instruction) in block.instructions.iter().enumerate().rev() {
            if let Instr::Ld8 { target, source } = instruction {
                live.swap_remove(&VirtualReg::R8(*source));
                graph
                    .node_to_data
                    .entry(VirtualReg::R8(*source))
                    .or_default()
                    .moves
                    .insert((block_id, index));
                graph
                    .node_to_data
                    .entry(VirtualReg::R8(*target))
                    .or_default()
                    .moves
                    .insert((block_id, index));
                moves.insert((block_id, index));
            }

            // its possible the register was never used, so we need
            // to ensure we add them to the live list so the correct
            // edges are added to the graph
            for reg in instruction_defs(instruction) {
                live.insert(reg);
            }
            for reg in instruction_defs(instruction) {
                for live_reg in live.iter().copied() {
                    graph.add_edge(live_reg, reg);
                }
            }

            // we are walking the instructions backwards, so the
            // definitions are no longer live after the instruction
            // and uses are live at least from this instruction.
            for reg in instruction_defs(instruction) {
                live.swap_remove(&reg);
            }
            for reg in instruction_uses(instruction) {
                live.insert(reg);
            }
        }
    }
    (graph, moves)
}

#[instrument(skip_all)]
pub fn calculate_block_liveness(code: &CodeSectionData) -> BlockLiveness {
    let mut block_to_uses = SecondaryMap::new();
    let mut block_to_defs = SecondaryMap::new();
    for (block_id, block) in &code.blocks {
        let mut uses = IndexSet::new();
        let mut defs = IndexSet::new();
        for instruction in &block.instructions {
            for reg in instruction_uses(instruction) {
                if !defs.contains(&reg) {
                    uses.insert(reg);
                }
            }

            for reg in instruction_defs(instruction) {
                defs.insert(reg);
            }
        }
        block_to_defs.insert(block_id, defs);
        block_to_uses.insert(block_id, uses);
    }

    let mut live_in: SecondaryMap<BlockId, IndexSet<VirtualReg>> = SecondaryMap::new();
    let mut live_out: SecondaryMap<BlockId, IndexSet<VirtualReg>> = SecondaryMap::new();
    for block_id in code.blocks.keys() {
        live_in.insert(block_id, IndexSet::new());
        live_out.insert(block_id, IndexSet::new());
    }

    let mut changed = true;
    while changed {
        changed = false;
        for (block_id, block) in code.blocks.iter() {
            let mut new_out = IndexSet::new();
            for next_block_id in block.next_blocks() {
                new_out.extend(live_in[next_block_id].iter().copied());
            }
            let mut new_in = block_to_uses[block_id].clone();
            for &t in &new_out {
                if !block_to_defs[block_id].contains(&t) {
                    new_in.insert(t);
                }
            }
            if new_out != live_out[block_id] {
                live_out.insert(block_id, new_out);
                changed = true;
            }
            if new_in != live_in[block_id] {
                live_in.insert(block_id, new_in);
                changed = true;
            }
        }
    }

    BlockLiveness { live_in, live_out }
}

pub struct BlockLiveness {
    pub live_in: SecondaryMap<BlockId, IndexSet<VirtualReg>>,
    pub live_out: SecondaryMap<BlockId, IndexSet<VirtualReg>>,
}

fn instruction_uses(instruction: &Instr) -> impl Iterator<Item = VirtualReg> + '_ {
    instruction
        .source_r8()
        .map(VirtualReg::R8)
        .chain(instruction.source_r16().map(VirtualReg::R16))
        .chain(instruction.flag_in().into_iter().map(VirtualReg::Flag))
}

fn instruction_defs(instruction: &Instr) -> impl Iterator<Item = VirtualReg> + '_ {
    instruction
        .target_r8s()
        .map(VirtualReg::R8)
        .chain(instruction.target_r16().into_iter().map(VirtualReg::R16))
        .chain(instruction.flags().map(VirtualReg::Flag))
}

fn restrict_from_instruction_targets(
    code: &CodeSectionData,
    instruction: &Instr,
    graph: &mut InterferenceGraph,
) {
    for reg in instruction_defs(instruction) {
        if let Some(color) = match reg {
            VirtualReg::R8(reg) => code.r8_registers[reg].color,
            VirtualReg::R16(reg) => code.r16_registers[reg].color,
            VirtualReg::Flag(reg) => code.flag_registers[reg].color,
        } {
            graph.set_precolored(reg, color);
        }
    }
}
