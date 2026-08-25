use crate::lir::{
    BlockId, CarryFlagId, CodeSectionData, Instr, Ld8Source, Operand8, R8Id, R16Id, ZeroFlagId,
};
use crate::reg::{Color, ColorSet};
use indexmap::IndexSet;
use slotmap::SecondaryMap;
use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VirtualReg {
    R8(R8Id),
    R16(R16Id),
    Zero(ZeroFlagId),
    Carry(CarryFlagId),
}

#[derive(Debug, Default)]
pub struct InterferenceGraph {
    edges: IndexSet<(VirtualReg, VirtualReg)>,
    node_to_edges: HashMap<VirtualReg, IndexSet<VirtualReg>>,
    node_to_data: HashMap<VirtualReg, NodeData>,
}

#[derive(Debug, Default)]
pub struct NodeData {
    pressure: u64,
    moves: IndexSet<(BlockId, usize)>,
    precolored: Option<Color>,
    permitted_colors: Option<ColorSet>,
}

impl InterferenceGraph {
    fn add_edge(&mut self, mut a: VirtualReg, mut b: VirtualReg) {
        assert_ne!(a, b, "self edges are not permitted");
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        if !self.edges.insert((a, b)) {
            return;
        }

        self.add_pressure_from(a, b);
        self.add_pressure_from(b, a);
    }

    fn add_pressure_from(&mut self, a: VirtualReg, b: VirtualReg) {
        if !self.is_precolored(a) {
            self.node_to_edges.entry(a).or_default().insert(b);
            self.node_to_data.entry(a).or_default().pressure += self.colors_blocked_by(a, b);
        }
    }

    fn is_precolored(&self, a: VirtualReg) -> bool {
        self.node_to_data
            .get(&a)
            .map(|data| data.precolored.is_some())
            .unwrap_or(false)
    }

    /// the number of colors of node `a` which are blocked by colors in node `b`
    fn colors_blocked_by(&self, a: VirtualReg, b: VirtualReg) -> u64 {
        self.permitted_colors(b)
            .into_iter()
            .map(|c1| {
                self.permitted_colors(a)
                    .into_iter()
                    .filter(|&c2| c1.conflicts(c2))
                    .count() as u64
            })
            .max()
            .unwrap_or(0)
    }

    /// iterate over the permitted colors for the given virtual register
    fn permitted_colors(&self, a: VirtualReg) -> ColorSet {
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
            VirtualReg::Zero(_) => Color::z(),
            VirtualReg::Carry(_) => Color::c(),
        }
    }
}

/// Returns the calculated interference graph + any moves detected
#[must_use]
pub fn build_interference_graph(
    code: &CodeSectionData,
) -> (InterferenceGraph, IndexSet<(BlockId, usize)>) {
    let mut graph = InterferenceGraph::default();
    let mut moves = IndexSet::new();
    let live_out = calculate_block_live_out(code);
    for (block_id, block) in code.blocks.iter() {
        let mut live: IndexSet<VirtualReg> = live_out[block_id].clone();
        for (index, instruction) in block.instructions.iter().enumerate().rev() {
            if let Instr::Ld8 {
                target,
                source: Ld8Source::Operand(Operand8::Direct(source)),
            } = instruction
            {
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

fn calculate_block_live_out(code: &CodeSectionData) -> SecondaryMap<BlockId, IndexSet<VirtualReg>> {
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

    live_out
}

fn instruction_uses(instruction: &Instr) -> impl Iterator<Item = VirtualReg> + '_ {
    instruction
        .source_r8()
        .map(VirtualReg::R8)
        .chain(instruction.source_r16().map(VirtualReg::R16))
        .chain(instruction.zero_flag_in().into_iter().map(VirtualReg::Zero))
        .chain(
            instruction
                .carry_flag_in()
                .into_iter()
                .map(VirtualReg::Carry),
        )
}

fn instruction_defs(instruction: &Instr) -> impl Iterator<Item = VirtualReg> + '_ {
    instruction
        .target_r8()
        .map(VirtualReg::R8)
        .chain(instruction.target_r16().into_iter().map(VirtualReg::R16))
        .chain(instruction.zero_flag().into_iter().map(VirtualReg::Zero))
        .chain(instruction.carry_flag().into_iter().map(VirtualReg::Carry))
}
