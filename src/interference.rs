use crate::lir::{
    Addr, BlockId, CodeSectionData, FlagId, HighAddr, Instr, Operand8, Terminator, VirtualReg,
};
use crate::reg::{Color, ColorSet};
use indexmap::map::Entry;
use indexmap::{IndexMap, IndexSet};
use slotmap::SecondaryMap;
use std::collections::{HashMap, VecDeque};
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
    constraints: IndexMap<VirtualReg, ConstraintKind>,
    precolored: Option<Color>,
    permitted_colors: Option<ColorSet>,
}

impl InterferenceGraph {
    fn set_precolored(&mut self, a: VirtualReg, color: Color) {
        assert!(
            a.allowed_color(color),
            "{a:?} cannot be precolored {color:?}",
        );
        let mut data = self.node_to_data.entry(a).or_default();
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
        if !data.constraints.is_empty() {
            self.set_permitted_colors_and_propagate_constraints(a, ColorSet::just(color));
            data = self.node_to_data.entry(a).or_default();
        }
        data.precolored = Some(color);
        data.pressure = usize::MAX;
        data.permitted_colors = None;
    }

    /// constrains a given register to the specified set. Returns true
    /// the set of candidates for the given register changes, false if
    /// no change, panics if register would become uncolorable.
    pub fn constrain_colors_to(&mut self, a: VirtualReg, set: ColorSet) -> bool {
        assert!(
            !set.is_empty(),
            "cannot constrain {a:?} to {set:?} as that would make it uncolorable"
        );
        let disallowed_colors = set
            .clone()
            .into_iter()
            .filter(|&color| !a.allowed_color(color))
            .collect::<ColorSet>();
        assert!(
            disallowed_colors.is_empty(),
            "cannot constrain {a:?} to {set:?} as it cannot be colored {disallowed_colors:?}"
        );

        let data = self.node_to_data.entry(a).or_default();
        if let Some(color) = data.precolored {
            assert!(
                set.contains(color),
                "cannot constrain {a:?} to {set:?} as it is already precolored as {color:?}",
            );
            return false;
        }
        let new_set = match data.permitted_colors.clone() {
            Some(existing) => {
                let merged = existing.clone() & set.clone();
                if merged == existing {
                    return false;
                }
                assert!(
                    !merged.is_empty(),
                    "cannot constrain {a:?} to {set:?} as that would make it uncolorable as it is already constrained to {existing:?}"
                );
                merged
            }
            None => set,
        };
        if data.constraints.is_empty() {
            data.permitted_colors = Some(new_set);
        } else {
            self.set_permitted_colors_and_propagate_constraints(a, new_set);
        }
        true
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

    /// Every node in the graph which has been precolored, including synthetic nodes (such as
    /// [`VirtualReg::Call`]) which have no backing storage in `CodeSectionData`. Used instead of
    /// [`CodeSectionData::registers`] to seed color assignment, since that only enumerates real,
    /// storage-backed registers.
    pub fn precolored(&self) -> impl Iterator<Item = (VirtualReg, Color)> + '_ {
        self.node_to_data
            .iter()
            .filter_map(|(&reg, data)| data.precolored.map(|color| (reg, color)))
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
            // Always precolored (see `set_precolored(_, Color::Call)` in
            // `build_interference_graph`), so this fallback is unreachable in practice.
            VirtualReg::Call(_) => ColorSet::just(Color::Call),
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
            VirtualReg::Call(_) => 1,
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

    #[must_use]
    pub fn constraints(&self, a: VirtualReg) -> Option<&IndexMap<VirtualReg, ConstraintKind>> {
        self.node_to_data
            .get(&a)
            .map(|data| &data.constraints)
            .filter(|&constraints| !constraints.is_empty())
    }

    pub fn add_constraint(&mut self, a: VirtualReg, b: VirtualReg, kind: ConstraintKind) {
        match self.node_to_data.entry(a).or_default().constraints.entry(b) {
            Entry::Occupied(entry) => {
                assert_eq!(
                    entry.get(),
                    &kind,
                    "Cannot constraint {a:?} and {b:?} to both {:?} and {:?}",
                    entry.get(),
                    kind
                );
            }
            Entry::Vacant(entry) => {
                entry.insert(kind);
            }
        }
        let kind = kind.inverse();
        match self.node_to_data.entry(b).or_default().constraints.entry(a) {
            Entry::Occupied(entry) => {
                assert_eq!(
                    entry.get(),
                    &kind,
                    "Cannot constraint {a:?} and {b:?} to both {:?} and {:?}",
                    entry.get(),
                    kind
                );
            }
            Entry::Vacant(entry) => {
                entry.insert(kind);
            }
        }
        self.set_permitted_colors_and_propagate_constraints(a, self.permitted_colors(a));
    }

    fn set_permitted_colors_and_propagate_constraints(
        &mut self,
        reg: VirtualReg,
        original_set: ColorSet,
    ) {
        let pending_updates = self.calculate_propagated_permitted_colors(reg, original_set.clone());
        let uncolorable_regs = pending_updates
            .iter()
            .filter_map(|(&reg, set)| if set.is_empty() { Some(reg) } else { None })
            .collect::<Vec<_>>();
        if !uncolorable_regs.is_empty() {
            let existing_colors = uncolorable_regs
                .iter()
                .copied()
                .map(|reg| (reg, self.permitted_colors(reg)))
                .collect::<IndexMap<_, _>>();
            panic!(
                "cannot constrain {reg:?} to {original_set:?} as that would make it the following registers uncolorable via constraints: {uncolorable_regs:?}. Existing colors = {existing_colors:?}, constraints = {:?}",
                self.constraints(reg)
            );
        }
        assert!(
            uncolorable_regs.is_empty(),
            "cannot constrain {reg:?} to {original_set:?} as that would make it the following registers uncolorable via constraints {uncolorable_regs:?}"
        );
        for (reg, set) in pending_updates {
            let data = self.node_to_data.entry(reg).or_default();
            if let Some(color) = data.precolored {
                assert_eq!(ColorSet::just(color), set);
            } else {
                data.permitted_colors = Some(set);
            }
        }
    }

    #[must_use]
    pub fn calculate_propagated_permitted_colors(
        &self,
        reg: VirtualReg,
        set: ColorSet,
    ) -> IndexMap<VirtualReg, ColorSet> {
        let mut propagated_updates = IndexMap::new();
        propagated_updates.insert(reg, set);
        let mut queue = VecDeque::new();
        queue.push_back(reg);
        while let Some(reg) = queue.pop_front() {
            let set = propagated_updates[&reg].clone();
            let Some(constraints) = self.constraints(reg) else {
                continue;
            };
            for (&dependent_reg, &kind) in constraints {
                let existing = propagated_updates
                    .get(&dependent_reg)
                    .cloned()
                    .unwrap_or_else(|| self.permitted_colors(dependent_reg));
                if existing.is_empty() {
                    continue;
                }
                let updated = match kind {
                    ConstraintKind::HighToR16 => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::A => Some(Color::AF),
                                Color::B => Some(Color::BC),
                                Color::D => Some(Color::DE),
                                Color::H => Some(Color::HL),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::LowToR16 => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::F => Some(Color::AF),
                                Color::C => Some(Color::BC),
                                Color::E => Some(Color::DE),
                                Color::L => Some(Color::HL),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::R16ToHigh => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::AF => Some(Color::A),
                                Color::BC => Some(Color::B),
                                Color::DE => Some(Color::D),
                                Color::HL => Some(Color::H),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::R16ToLow => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::AF => Some(Color::F),
                                Color::BC => Some(Color::C),
                                Color::DE => Some(Color::E),
                                Color::HL => Some(Color::L),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::HighToLow => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::A => Some(Color::F),
                                Color::B => Some(Color::C),
                                Color::D => Some(Color::E),
                                Color::H => Some(Color::L),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::LowToHigh => {
                        let new_set = set
                            .clone()
                            .into_iter()
                            .filter_map(|color| match color {
                                Color::F => Some(Color::A),
                                Color::C => Some(Color::B),
                                Color::E => Some(Color::D),
                                Color::L => Some(Color::H),
                                _ => None,
                            })
                            .collect::<ColorSet>();
                        existing.clone() & new_set
                    }
                    ConstraintKind::R16OrHLToAOrR8 => {
                        if !set.contains(Color::HL) {
                            existing.clone() & ColorSet::just(Color::A)
                        } else {
                            existing.clone()
                        }
                    }
                    ConstraintKind::R8OrAToHLOrR16 => {
                        if !set.contains(Color::A) {
                            existing.clone() & ColorSet::just(Color::HL)
                        } else {
                            existing.clone()
                        }
                    }
                };
                if updated != existing {
                    propagated_updates.insert(dependent_reg, updated);
                    queue.push_back(dependent_reg);
                }
            }
        }
        propagated_updates
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
    for reg in code.registers() {
        if let Some(color) = code.register_data(reg).color {
            graph.set_precolored(reg, color);
        }
    }

    for block in code.blocks.values() {
        for instruction in &block.instructions {
            restrict_from_instruction(code, instruction, &mut graph);
        }
        restrict_from_terminator(&block.terminator, &mut graph);
    }

    let mut moves = IndexSet::new();
    for (block_id, block) in code.blocks.iter() {
        let mut live: IndexSet<VirtualReg> = live_out[block_id].clone();

        // A call clobbers every register (see `Color::Call`), so anything live across it - i.e.
        // still live *after* the call returns - must be forced to spill rather than trust the
        // callee (which is register-allocated entirely independently, in its own code section)
        // to leave any particular color untouched. We add this edge against `live` before
        // folding in the terminator's own `register_uses()` (call args / the callee pointer for
        // `CallR16`), since those die going into the call and aren't live across it.
        if matches!(
            block.terminator,
            Terminator::Call { .. } | Terminator::CallIf { .. } | Terminator::CallR16 { .. }
        ) {
            let call_reg = VirtualReg::Call(block_id);
            graph.set_precolored(call_reg, Color::Call);
            for &reg in &live {
                graph.add_edge(reg, call_reg);
            }
        }

        for reg in block.terminator.register_uses() {
            live.insert(reg);
        }
        for (index, instruction) in block.instructions.iter().enumerate().rev() {
            if let Some((target, source)) = instruction.as_move() {
                // temporarily remove the source from the live set so that an edge is
                // not created between the source and target as this would prevent coalescing.
                // it will be added back into the live set end of the loop block
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
            } else if matches!(instruction, Instr::Join { .. } | Instr::Split { .. }) {
                // it is completely fine for the split registers or the joined registers
                // as they represent the same register pair and same value stored on it.
                // as a result, we remove the sources from the live set to prevent edges
                // being added between them. The uses will be added back into the live set
                // at the end of the loop block
                for reg in instruction.register_uses() {
                    live.swap_remove(&reg);
                }
            }

            // its possible the register was never used, so we need
            // to ensure we add them to the live list so the correct
            // edges are added to the graph
            for reg in instruction.register_new_defs() {
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
            for reg in instruction.register_new_defs() {
                live.swap_remove(&reg);
            }
            for reg in instruction.register_uses() {
                live.insert(reg);
            }
        }
    }
    validate_initial_coloring(&graph, code);
    (graph, moves)
}

fn validate_initial_coloring(graph: &InterferenceGraph, code: &CodeSectionData) {
    for (reg, data) in &graph.node_to_data {
        if let Some(permitted_colors) = data.permitted_colors.as_ref().cloned() {
            assert!(
                !permitted_colors.is_empty(),
                "{reg:?} has an initial state of no permitted colors. precolor = {:?}",
                data.precolored
            );
        }
    }
    for &(a, b) in &graph.edges {
        let (Some(a_color), Some(b_color)) = (graph.precolor(a), graph.precolor(b)) else {
            continue;
        };
        // precolored registers are not candidates for spilling within the register allocation
        // algorithm, so performing this check up front ensures that uncolorable scenarios or ones
        // which would cause issues fail upfront.
        if a_color.conflicts(b_color) {
            for (block_id, block) in code.blocks.iter() {
                eprintln!(".{block_id}:");
                for (index, instr) in block.instructions.iter().enumerate() {
                    eprint!("{index}: {instr:?} #");
                    if instr.register_new_defs().any(|r| r == a) {
                        eprint!(" DEFINED a");
                    }
                    if instr.register_new_defs().any(|r| r == b) {
                        eprint!(" DEFINED b");
                    }
                    if instr.register_uses().any(|r| r == a) {
                        eprint!(" USED a");
                    }
                    if instr.register_uses().any(|r| r == b) {
                        eprint!(" USED b");
                    }
                    eprintln!();
                }
                eprint!("{}: {:?} #", block.instructions.len(), block.terminator);
                if block.terminator.register_uses().any(|r| r == a) {
                    eprint!(" USED a");
                }
                println!();
            }
            let a_defined = if a.is_synthetic() {
                Vec::new()
            } else {
                code.register_data(a).defined.iter().collect::<Vec<_>>()
            };
            let b_defined = if b.is_synthetic() {
                Vec::new()
            } else {
                code.register_data(b).defined.iter().collect::<Vec<_>>()
            };
            panic!(
                "{a:?}(defined={a_defined:?}) and {b:?}(defined={b_defined:?}) are both precolored to conflicting colors ({a_color:?}, {b_color:?}) but are simultaneously live",
            );
        }
    }
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

            for reg in instruction.register_new_defs() {
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
    // ideally we would set the precolored here, however we can end up with dandling
    // register uses, so we have to enumerate all the registers from the slot map rather
    // than set if from instruction.register_defs();
    // Currently set in build_interference_graph()

    // constrain all registers, even the precolored ones, as we expect a panic
    // if we attempt to constrain to a color-set which does not contain the precolor
    // and that would indicate a bug in the code
    match instruction {
        Instr::Nop | Instr::Breakpoint | Instr::LdSPValue { .. } => {}
        Instr::Ld8 { target, source } => {
            // `LD r8,r8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r8());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
        }
        Instr::Res { source, .. } | Instr::Set { source, .. } => {
            // `RES u3,r8`
            // `SET u3,r8`
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
            if let Addr::Reg(source_addr) = source_addr {
                graph.add_constraint(
                    VirtualReg::from(*target),
                    VirtualReg::from(*source_addr),
                    ConstraintKind::R8OrAToHLOrR16,
                );
                graph.constrain_colors_to(VirtualReg::from(*source_addr), source_set);
            }
            graph.constrain_colors_to(VirtualReg::from(*target), target_set);
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
            source_addr,
        }
        | Instr::Ld8Dec {
            target,
            source_addr,
        } => {
            // `LD A,[HLI]`
            // `LD A,[HLD]`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*source_addr), Color::r16_ptr());
        }
        Instr::Ld16 { target, .. } => {
            // `LD r16,n16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16());
        }
        Instr::Store { addr, source } => {
            // `LD [HL],r8`
            // `LD [r16],A`
            // `LD [n16],A`
            let (target_set, source_set) = if let Addr::Reg(r16) = addr
                && code.r16_registers[*r16].color == Some(Color::HL)
            {
                (Color::r16_ptr(), Color::r8())
            } else {
                (Color::r16(), Color::acc())
            };
            if let Addr::Reg(target_addr) = addr {
                graph.add_constraint(
                    VirtualReg::from(*target_addr),
                    VirtualReg::from(*source),
                    ConstraintKind::R16OrHLToAOrR8,
                );
                graph.constrain_colors_to(VirtualReg::from(*target_addr), target_set);
            }
            graph.constrain_colors_to(VirtualReg::from(*source), source_set);
        }
        Instr::StoreHigh { addr, source } => {
            // `LDH [n16],A`
            // `LDH [C],A`
            if let HighAddr::Reg(target_addr) = addr {
                graph.constrain_colors_to(VirtualReg::from(*target_addr), Color::r8_high_ptr());
            }
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::StoreValue { addr, .. } => {
            // `LD [HL],n8`
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
        }
        Instr::StoreInc { addr, source } | Instr::StoreDec { addr, source } => {
            // `LD [HLI],A`
            // `LD [HLD],A`
            graph.constrain_colors_to(VirtualReg::from(*addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::Adc {
            z,
            c,
            left,
            right,
            c_in,
        }
        | Instr::Sbc {
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
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::acc());
            restrict_from_operand(right, graph);
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::Add { z, c, left, right }
        | Instr::Sub { z, c, left, right }
        | Instr::And { z, c, left, right }
        | Instr::Or { z, c, left, right }
        | Instr::Xor { z, c, left, right } => {
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
        Instr::Dec { z, source } | Instr::Inc { z, source } => {
            // `DEC r8`
            // `INC r8`
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
        Instr::Cpl { source } => {
            // `CPL`
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
        Instr::Rl { z, c, source, c_in } | Instr::Rr { z, c, source, c_in } => {
            // `RL r8`
            // `RR r8`
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
        Instr::Rla { z, c, source, c_in } | Instr::Rra { z, c, source, c_in } => {
            // `RLA`
            // `RRA`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
        }
        Instr::Rlca { z, c, source } | Instr::Rrca { z, c, source } => {
            // `RLCA`
            // `RRCA`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*source), Color::acc());
        }
        Instr::Rlc { z, c, source }
        | Instr::Rrc { z, c, source }
        | Instr::Sla { z, c, source }
        | Instr::Sra { z, c, source }
        | Instr::Srl { z, c, source }
        | Instr::Swap { z, c, source } => {
            // `RLC r8`
            // `RRC r8`
            // `SLA r8`
            // `SRA r8`
            // `SRL r8`
            // `SWAP r8`
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
        Instr::Add16 { c, left, right } => {
            // `ADD HL,r16`
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*left), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*right), Color::r16());
        }
        Instr::AddSP { z, c, .. } => {
            // `ADD SP e8`
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::LdSPE8 { z, c, target, .. } => {
            // `LD HL SP+e8`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
        }
        Instr::Dec16 { source } | Instr::Inc16 { source } => {
            // `DEC r16`
            // `INC r16`
            graph.constrain_colors_to(VirtualReg::from(*source), Color::r16());
        }
        Instr::PushR16 { source: target } | Instr::PopR16 { target } => {
            // `PUSH r16`
            // `POP r16`
            graph.constrain_colors_to(VirtualReg::from(*target), Color::r16());
        }
        Instr::PushAF { source, z_in, c_in } => {
            // `PUSH AF`
            if let Some(source) = source {
                graph.constrain_colors_to(VirtualReg::from(*source), Color::r8());
            }
            if let Some(z_in) = z_in {
                graph.constrain_colors_to(VirtualReg::from(*z_in), Color::z());
            }
            if let Some(c_in) = c_in {
                graph.constrain_colors_to(VirtualReg::from(*c_in), Color::c());
            }
        }
        Instr::PopAF { target, z, c } => {
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
            graph.add_constraint(
                VirtualReg::from(*high),
                VirtualReg::from(*low),
                ConstraintKind::HighToLow,
            );
            graph.add_constraint(
                VirtualReg::from(*high),
                VirtualReg::from(*r16),
                ConstraintKind::HighToR16,
            );
            graph.add_constraint(
                VirtualReg::from(*low),
                VirtualReg::from(*r16),
                ConstraintKind::LowToR16,
            );
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
        Instr::FreshA { target } => {
            graph.constrain_colors_to(VirtualReg::from(*target), Color::acc());
        }
        Instr::BulkCopy {
            z,
            c,
            target_addr,
            source_addr,
            count,
            scratch,
        } => {
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*target_addr), ColorSet::just(Color::DE));
            graph.constrain_colors_to(VirtualReg::from(*source_addr), Color::r16_ptr());
            graph.constrain_colors_to(VirtualReg::from(*count), ColorSet::just(Color::BC));
            graph.constrain_colors_to(VirtualReg::from(*scratch), Color::acc());
        }
        Instr::BulkFill {
            z,
            c,
            addr,
            count,
            value,
            scratch,
        } => {
            graph.constrain_colors_to(VirtualReg::from(*z), Color::z());
            graph.constrain_colors_to(VirtualReg::from(*c), Color::c());
            graph.constrain_colors_to(VirtualReg::from(*addr), ColorSet::just(Color::HL));
            graph.constrain_colors_to(VirtualReg::from(*count), ColorSet::just(Color::BC));
            graph.constrain_colors_to(VirtualReg::from(*value), Color::acc());
            graph.constrain_colors_to(VirtualReg::from(*scratch), ColorSet::just(Color::D));
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

fn restrict_from_terminator(terminator: &Terminator, graph: &mut InterferenceGraph) {
    match terminator {
        Terminator::CallIf { source, .. }
        | Terminator::JpIf { source, .. }
        | Terminator::RetIf { source, .. } => {
            match source {
                FlagId::Zero(flag) => {
                    graph.constrain_colors_to(VirtualReg::from(*flag), Color::z())
                }
                FlagId::Carry(flag) => {
                    graph.constrain_colors_to(VirtualReg::from(*flag), Color::c())
                }
            };
        }
        Terminator::CallR16 { callee, .. } => {
            // `CallR16` is always emitted as `JP HL`, so its callee register must be HL exactly.
            graph.constrain_colors_to(VirtualReg::from(*callee), Color::r16_ptr());
        }
        Terminator::Call { .. }
        | Terminator::Jp { .. }
        | Terminator::JpFn { .. }
        | Terminator::Ret
        | Terminator::Reti
        | Terminator::Halt { .. } => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    HighToR16,
    LowToR16,
    R16ToHigh,
    R16ToLow,
    HighToLow,
    LowToHigh,
    R16OrHLToAOrR8,
    R8OrAToHLOrR16,
}

impl ConstraintKind {
    #[must_use]
    pub fn inverse(self) -> Self {
        match self {
            ConstraintKind::HighToR16 => ConstraintKind::R16ToHigh,
            ConstraintKind::LowToR16 => ConstraintKind::R16ToLow,
            ConstraintKind::R16ToHigh => ConstraintKind::HighToR16,
            ConstraintKind::R16ToLow => ConstraintKind::LowToR16,
            ConstraintKind::HighToLow => ConstraintKind::LowToHigh,
            ConstraintKind::LowToHigh => ConstraintKind::HighToLow,
            ConstraintKind::R16OrHLToAOrR8 => ConstraintKind::R8OrAToHLOrR16,
            ConstraintKind::R8OrAToHLOrR16 => ConstraintKind::R16OrHLToAOrR8,
        }
    }
}
