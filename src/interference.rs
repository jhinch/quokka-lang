use crate::lir::{Addr, BlockId, CodeSectionData, HighAddr, Instr, Operand8, VirtualReg};
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
        data.permitted_colors = Some(match data.permitted_colors.take() {
            Some(existing) => existing & set,
            None => set,
        });
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
            VirtualReg::Zero(_) => Color::z(),
            VirtualReg::Carry(_) => Color::c(),
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
            VirtualReg::Zero(_) => Color::z().len(),
            VirtualReg::Carry(_) => Color::c().len(),
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
            restrict_from_instruction(code, instruction, &mut graph);
        }
    }

    let mut moves = IndexSet::new();
    for (block_id, block) in code.blocks.iter() {
        let mut live: IndexSet<VirtualReg> = live_out[block_id].clone();
        for reg in block.terminator.register_uses() {
            live.insert(reg);
        }
        for (index, instruction) in block.instructions.iter().enumerate().rev() {
            if let Some((target, source)) = instruction.as_move() {
                live.swap_remove(&source);
                graph
                    .node_to_data
                    .entry(source)
                    .or_default()
                    .moves
                    .insert((block_id, index));
                graph
                    .node_to_data
                    .entry(target)
                    .or_default()
                    .moves
                    .insert((block_id, index));
                moves.insert((block_id, index));
            }

            // its possible the register was never used, so we need
            // to ensure we add them to the live list so the correct
            // edges are added to the graph
            for reg in instruction.register_defs() {
                live.insert(reg);
            }
            for reg in instruction.register_defs() {
                for live_reg in live.iter().copied() {
                    if live_reg != reg {
                        graph.add_edge(live_reg, reg);
                    }
                }
            }

            // we are walking the instructions backwards, so the
            // definitions are no longer live after the instruction
            // and uses are live at least from this instruction.
            for reg in instruction.register_defs() {
                live.swap_remove(&reg);
            }
            for reg in instruction.register_uses() {
                live.insert(reg);
            }
        }
    }
    if cfg!(debug_assertions) {
        for (reg, data) in &graph.node_to_data {
            if let Some(permitted_colors) = data.permitted_colors.as_ref().cloned() {
                assert!(
                    !permitted_colors.is_empty(),
                    "{reg:?} has an initial state of no permitted colors. precolor = {:?}",
                    data.precolored
                );
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
            for reg in instruction.register_uses() {
                if !defs.contains(&reg) {
                    uses.insert(reg);
                }
            }

            for reg in instruction.register_defs() {
                defs.insert(reg);
            }
        }
        for reg in block.terminator.register_uses() {
            if !defs.contains(&reg) {
                uses.insert(reg);
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

fn restrict_from_instruction(
    code: &CodeSectionData,
    instruction: &Instr,
    graph: &mut InterferenceGraph,
) {
    for reg in instruction.register_defs() {
        if let Some(color) = match reg {
            VirtualReg::R8(reg) => code.r8_registers[reg].color,
            VirtualReg::R16(reg) => code.r16_registers[reg].color,
            VirtualReg::Zero(reg) => code.zero_registers[reg].color,
            VirtualReg::Carry(reg) => code.carry_registers[reg].color,
        } {
            graph.set_precolored(reg, color);
        }
    }
    // constrain all registers, even the precolored ones, as we expect a panic
    // if we attempt to constrain to a color-set which does not contain the precolor
    // and that would indicate a bug in the code
    match instruction {
        Instr::Nop => {}
        Instr::Ld8 { target, source }
        | Instr::Res { target, source, .. }
        | Instr::Set { target, source, .. } => {
            // `LD r8,r8`
            // `RES u3,r8`
            // `SET u3,r8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
        }
        Instr::Ld8Value { target, .. } => {
            // `LD r8,n8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
        }
        Instr::Ld8Ptr {
            target,
            source_addr,
        } => {
            // `LD r8,[HL]`
            // `LD A,[r16]`
            // `LD A,[n16]`
            let (target_set, source_set) = if let Addr::Reg(r16) = source_addr
                && code.r16_registers[*r16].color == Some(Color::HL)
            {
                (Color::r8(), Color::r16_ptr())
            } else {
                (Color::acc(), Color::r16())
            };
            graph.constrain_colors_to(VirtualReg::from(*target), target_set);
            if let Addr::Reg(source_addr) = source_addr {
                graph.constrain_colors_to(VirtualReg::from(*source_addr), source_set);
            }
        }
        Instr::Ld8HighPtr {
            target,
            source_addr,
        } => {
            // `LDH A,[n16]`
            // `LDH A,[C]`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            if let HighAddr::Reg(source_addr) = source_addr {
                graph.constrain_colors_to(VirtualReg::from(*source_addr), Color::r8_high_ptr());
            }
        }
        Instr::Ld8Inc {
            target,
            revised_addr,
            source_addr,
        }
        | Instr::Ld8Dec {
            target,
            revised_addr,
            source_addr,
        } => {
            // `LD A,[HLI]`
            // `LD A,[HLD]`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*revised_addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*source_addr), Color::r16_ptr());
        }
        Instr::Ld16 { target, .. } => {
            // `LD r16,n16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16());
        }
        Instr::Store {
            target_addr,
            source,
        } => {
            // `LD [HL],r8`
            // `LD [r16],A`
            // `LD [n16],A`
            let (target_set, source_set) = if let Addr::Reg(r16) = target_addr
                && code.r16_registers[*r16].color == Some(Color::HL)
            {
                (Color::r16_ptr(), Color::r8())
            } else {
                (Color::r16(), Color::acc())
            };
            if let Addr::Reg(target_addr) = target_addr {
                graph.constrain_colors_to(VirtualReg::from(*target_addr), target_set);
            }
            graph.constrain_colors_to(VirtualReg::from(*source), source_set);
        }
        Instr::StoreHigh {
            addr: target_addr,
            source,
        } => {
            // `LDH [n16],A`
            // `LDH [C],A`
            if let HighAddr::Reg(target_addr) = target_addr {
                graph.constrain_colors_to(VirtualReg::from(*target_addr), Color::r8_high_ptr());
            }
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::StoreValue {
            addr: target_addr, ..
        } => {
            // `LD [HL],n8`
            graph.constrain_colors_to(VirtualReg::from(*target_addr), Color::r16_ptr());
        }
        Instr::StoreInc {
            revised_addr,
            addr: target_addr,
            source,
        }
        | Instr::StoreDec {
            revised_addr,
            addr: target_addr,
            source,
        } => {
            // `LD [HLI],A`
            // `LD [HLD],A`
            graph.constrain_colors_to(VirtualReg::from(*revised_addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*target_addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::Adc {
            target,
            z,
            c,
            left,
            right,
            c_in,
        }
        | Instr::Sbc {
            target,
            z,
            c,
            left,
            right,
            c_in,
        } => {
            // `ADC A,r8`
            // `ADC A,[HL]`
            // `ADC A,n8`
            // `SBC A,r8`
            // `SBC A,[HL]`
            // `SBC A,n8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::acc());
            restrict_from_operand(right, graph);
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::Add {
            target,
            z,
            c,
            left,
            right,
        }
        | Instr::Sub {
            target,
            z,
            c,
            left,
            right,
        }
        | Instr::And {
            target,
            z,
            c,
            left,
            right,
        }
        | Instr::Or {
            target,
            z,
            c,
            left,
            right,
        }
        | Instr::Xor {
            target,
            z,
            c,
            left,
            right,
        } => {
            // `ADD A,r8`
            // `ADD A,[HL]`
            // `ADD A,n8`
            // `SUB A,r8`
            // `SUB A,[HL]`
            // `SUB A,n8`
            // `AND A,r8`
            // `AND A,[HL]`
            // `AND A,n8`
            // `OR A,r8`
            // `OR A,[HL]`
            // `OR A,n8`
            // `XOR A,r8`
            // `XOR A,[HL]`
            // `XOR A,n8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::acc());
            restrict_from_operand(right, graph);
        }
        Instr::Cp { z, c, left, right } => {
            // `CP A,r8`
            // `CP A,[HL]`
            // `CP A,n8`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::acc());
            restrict_from_operand(right, graph);
        }
        Instr::Dec { target, z, source } | Instr::Inc { target, z, source } => {
            // `DEC r8`
            // `INC r8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
        }
        Instr::DecPtr { z, addr }
        | Instr::IncPtr { z, addr }
        | Instr::BitPtr {
            z,
            source_addr: addr,
            ..
        } => {
            // `DEC [HL]`
            // `INC [HL]`
            // `BIT u3,[HL]`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
        }
        Instr::Cpl { target, source } => {
            // `CPL`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::Bit { z, source, .. } => {
            // `BIT u3,r8`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
        }
        Instr::ResPtr { addr, .. } | Instr::SetPtr { addr, .. } => {
            // `RES u3,[HL]`
            // `SET u3,[HL]`
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
        }
        Instr::Rl {
            target,
            z,
            c,
            source,
            c_in,
        }
        | Instr::Rr {
            target,
            z,
            c,
            source,
            c_in,
        } => {
            // `RL r8`
            // `RLA`
            // `RR r8`
            // `RRA`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::RlPtr { z, c, addr, c_in } | Instr::RrPtr { z, c, addr, c_in } => {
            // `RL [HL]`
            // `RR [HL]`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::Rlc {
            target,
            z,
            c,
            source,
        }
        | Instr::Rrc {
            target,
            z,
            c,
            source,
        }
        | Instr::Sla {
            target,
            z,
            c,
            source,
        }
        | Instr::Sra {
            target,
            z,
            c,
            source,
        }
        | Instr::Srl {
            target,
            z,
            c,
            source,
        }
        | Instr::Swap {
            target,
            z,
            c,
            source,
        } => {
            // `RLC r8`
            // `RLCA`
            // `RRC r8`
            // `RRCA`
            // `SLA r8`
            // `SRA r8`
            // `SRL r8`
            // `SWAP r8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
        }
        Instr::RlcPtr { z, c, addr }
        | Instr::RrcPtr { z, c, addr }
        | Instr::SlaPtr { z, c, addr }
        | Instr::SraPtr { z, c, addr }
        | Instr::SrlPtr { z, c, addr }
        | Instr::SwapPtr { z, c, addr } => {
            // `RLC [HL]`
            // `RRC [HL]`
            // `SLA [HL]`
            // `SRA [HL]`
            // `SRL [HL]`
            // `SWAP [HL]`
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::Ccf { c, c_in } | Instr::MoveCarry { c, c_in } => {
            // `CCF`
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::Scf { c } => {
            // `SCF`
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::Add16 {
            target,
            c,
            left,
            right,
        } => {
            // `ADD HL,r16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*right), Color::r16());
        }
        Instr::AddSP { z, c, .. } => {
            // `ADD SP e8`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::LdSPE8 { c, target, .. } => {
            // `LD HL SP+e8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::Dec16 { target, source } | Instr::Inc16 { target, source } => {
            // `DEC r16`
            // `INC r16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r16());
        }
        Instr::PushR16 { source: target } | Instr::PopR16 { target } => {
            // `PUSH r16`
            // `POP r16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16());
        }
        Instr::PushAF {
            source: target,
            z_in: z,
            c_in: c,
        }
        | Instr::PopAF { target, z, c } => {
            // `PUSH AF`
            // `POP AF`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::Join {
            target: r16,
            high_in: high,
            low_in: low,
        }
        | Instr::Split {
            high,
            low,
            source: r16,
        } => {
            graph.constrain_colors_to(VirtualReg::from(*low), Color::r8_low());
            graph.constrain_colors_to(VirtualReg::from(*high), Color::r8_high());
            graph.constrain_colors_to(VirtualReg::from(*r16), Color::r16_stack());
        }
        Instr::MoveZero { z, z_in } => {
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*z_in), Color::z());
        }
        Instr::LdReturn8 { target } => {
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
        }
        Instr::LdReturn16 { target } => {
            graph.constrain_colors_to(VirtualReg::from(*target), ColorSet::just(Color::BC));
        }
    }
}

fn restrict_from_operand(right: &Operand8, graph: &mut InterferenceGraph) {
    match right {
        Operand8::Ptr(right) => {
            graph.constrain_colors_to(VirtualReg::from(*right), Color::r16_ptr());
        }
        Operand8::Reg(right) => {
            graph.constrain_colors_to(VirtualReg::from(*right), Color::r8());
        }
        Operand8::Literal(_) => {}
    }
}
