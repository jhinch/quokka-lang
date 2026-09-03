use crate::interference::{
    BlockLiveness, InterferenceGraph, build_interference_graph, calculate_block_liveness,
};
use crate::lir::{
    AId, Addr, Block, BlockId, CarryId, CodeSectionData, HLId, HighAddr, Instr, Lir, Operand8,
    R8Id, R16Id, Register, SectionData, Terminator, VirtualReg, ZeroId,
};
use crate::reg::{Color, ColorSet};
use indexmap::{IndexMap, IndexSet};
use quokka_lang_slotkeyset::{SparseSlotKeySet, StableSparseSlotKeySet};
use slotmap::SparseSecondaryMap;
use std::collections::{HashMap, VecDeque};
use tracing::instrument;

#[instrument(skip_all)]
pub fn allocate_registers(lir: &mut Lir) {
    for section in lir.sections.values_mut() {
        if let SectionData::Code(code) = &mut section.data {
            let colors = allocate_registers_for_code(code);
            for (reg, color) in colors {
                match reg {
                    VirtualReg::R8(reg) => code.r8_registers[reg].color = Some(color),
                    VirtualReg::R16(reg) => code.r16_registers[reg].color = Some(color),
                    VirtualReg::Zero(reg) => code.zero_registers[reg].color = Some(color),
                    VirtualReg::Carry(reg) => code.carry_registers[reg].color = Some(color),
                }
            }
            cleanup_redundant_instructions(code);
        }
    }
}

#[instrument(skip_all)]
fn allocate_registers_for_code(code: &mut CodeSectionData) -> HashMap<VirtualReg, Color> {
    let mut spill_slots = SpillSlots::default();
    let max_iterations = code.r8_registers.len()
        + code.r16_registers.len()
        + code.zero_registers.len()
        + code.carry_registers.len()
        + 100;
    let mut previous_spills = IndexSet::new();
    for i in 0..max_iterations {
        match allocate_registers_round(i, code, &mut spill_slots) {
            Ok(colors) => return colors,
            Err(spills) if spills == previous_spills => panic!(
                "Failed to allocate registers after {i} previous iterations due to successive spills for same registers {spills:?}"
            ),
            Err(spills) => {
                previous_spills = spills;
            }
        }
    }
    panic!("Failed to allocate registers after {max_iterations} iterations")
}

#[instrument(skip_all, fields(i = %i))]
fn allocate_registers_round(
    i: usize,
    code: &mut CodeSectionData,
    spill_slots: &mut SpillSlots,
) -> Result<HashMap<VirtualReg, Color>, IndexSet<VirtualReg>> {
    let block_liveness = calculate_block_liveness(code);
    match try_allocate_registers(code, &block_liveness) {
        Ok(colors) => Ok(colors),
        Err(failure) => {
            let spills = failure.spills.clone();
            rewrite_code(code, spill_slots, failure);
            Err(spills)
        }
    }
}

#[instrument(skip_all)]
fn try_allocate_registers(
    code: &CodeSectionData,
    block_liveness: &BlockLiveness,
) -> Result<HashMap<VirtualReg, Color>, Box<AllocationFailure>> {
    let (graph, moves) = build_interference_graph(code, block_liveness);
    let allocator = RegAllocator {
        graph,
        worklist_moves: moves,
        ..Default::default()
    };
    allocator.run(code)
}

fn calculate_function_blocks(code: &CodeSectionData) -> FunctionBlocks {
    let mut result = FunctionBlocks::default();
    for entry in code.functions.keys() {
        let mut blocks = SparseSlotKeySet::new();
        let mut exits = StableSparseSlotKeySet::new();
        let mut queue = VecDeque::new();
        queue.push_back(entry);
        while let Some(block) = queue.pop_front() {
            if blocks.contains(block) {
                continue;
            }
            blocks.insert(block);
            result.block_to_function.insert(block, entry);
            queue.extend(code.blocks[block].next_blocks());
            if matches!(
                code.blocks[block].terminator,
                Terminator::Ret | Terminator::RetIf { .. }
            ) {
                exits.insert(block);
            }
        }
        result.function_to_exits.insert(entry, exits);
    }
    result
}

#[derive(Default)]
pub struct RegAllocator {
    graph: InterferenceGraph,

    // A virtual reg is to be in only one worklist at a time
    simplify_worklist: IndexSet<VirtualReg>,
    freeze_worklist: IndexSet<VirtualReg>,
    spill_worklist: IndexSet<VirtualReg>,

    /// registers which require spilling
    spilled_nodes: IndexSet<VirtualReg>,
    /// registers which have been coalesced
    coalesced_nodes: IndexSet<VirtualReg>,
    /// registers which have been colored
    colored_nodes: IndexSet<VirtualReg>,
    /// Selected registers to be colored
    select_stack: VecDeque<VirtualReg>,
    on_stack: IndexSet<VirtualReg>,

    /// worklist of moves for consideration for coalescing
    worklist_moves: IndexSet<(BlockId, usize)>,
    // A move instruction is to be in only one move set at a time
    /// Moves which have been coalesced to the same register
    coalesced_moves: IndexSet<(BlockId, usize)>,
    /// Moves which have been provably required due to their registers interfering
    constrained_moves: IndexSet<(BlockId, usize)>,
    /// Moves which have been ruled out from being candidates for coalescing based on heuristic
    frozen_moves: IndexSet<(BlockId, usize)>,
    /// moves which are in a pending state but have not been added back to the worklist yet
    active_moves: IndexSet<(BlockId, usize)>,

    alias: HashMap<VirtualReg, VirtualReg>,
}

impl RegAllocator {
    pub fn run(
        mut self,
        code: &CodeSectionData,
    ) -> Result<HashMap<VirtualReg, Color>, Box<AllocationFailure>> {
        self.initialize_work(code);
        loop {
            if !self.simplify_worklist.is_empty() {
                self.simplify();
            } else if !self.worklist_moves.is_empty() {
                self.coalesce(code);
            } else if !self.freeze_worklist.is_empty() {
                self.freeze(code);
            } else if !self.spill_worklist.is_empty() {
                self.select_spill(code);
            } else {
                break;
            }
        }
        self.assign_colors(code)
    }

    fn initialize_work(&mut self, code: &CodeSectionData) {
        for reg in code
            .r8_registers
            .keys()
            .map(VirtualReg::R8)
            .chain(code.r16_registers.keys().map(VirtualReg::R16))
        {
            if self.graph.is_precolored(reg) {
                continue;
            }
            if self.graph.pressure_of(reg) >= self.graph.capacity_of(reg) {
                self.spill_worklist.insert(reg);
            } else if self.has_related_moves(reg) {
                self.freeze_worklist.insert(reg);
            } else {
                self.simplify_worklist.insert(reg);
            }
        }
    }

    /// removes one register from the simplify list and puts it on the selection stack,
    /// decreasing the pressure on adjacent nodes, and placing those neighbors in either
    /// the simplify or freeze lists.
    fn simplify(&mut self) {
        let Some(reg) = self.simplify_worklist.pop() else {
            return;
        };
        self.select_stack.push_front(reg);
        self.on_stack.insert(reg);
        for adjacent in self.get_edges_ignoring_processed(reg) {
            let delta = self.graph.blocked_colors_count(adjacent, reg);
            self.decrease_pressure(adjacent, delta);
        }
    }

    /// take a move instruction from the move worklist and determine if its source and
    /// target registers can be coalesced. It will either be combined using `combine` and
    /// deemed coalesced, it will be ruled out from further consideration for coalescing
    /// (placed into `constrained`), or left pending in the `active_moves` list.
    fn coalesce(&mut self, code: &CodeSectionData) {
        let Some(mv) = self.worklist_moves.pop() else {
            return;
        };
        let (block, index) = mv;
        let Some((a, b)) = code.blocks[block].instructions[index].as_move() else {
            unimplemented!(
                "{:?} is not a move but it is stored in the moves worklist",
                code.blocks[block].instructions[index]
            );
        };
        let mut a = self.get_alias(a);
        let mut b = self.get_alias(b);
        // Preference keeping a precolored node
        if self.graph.is_precolored(b) {
            std::mem::swap(&mut a, &mut b);
        }

        if a == b {
            self.coalesced_moves.insert(mv);
            self.add_back_to_simplify_if_within_capacity(a);
            return;
        }
        if self.graph.is_precolored(b) || self.graph.has_edge(a, b) {
            self.constrained_moves.insert(mv);
            self.add_back_to_simplify_if_within_capacity(a);
            self.add_back_to_simplify_if_within_capacity(b);
            return;
        }

        let permitted_a = self.graph.permitted_colors(a);
        let permitted_b = self.graph.permitted_colors(b);
        let merged = permitted_a & permitted_b;

        if self.george_test(a, b, &merged) || self.briggs_test(a, b, &merged) {
            self.coalesced_moves.insert(mv);
            self.combine(a, b);
            self.add_back_to_simplify_if_within_capacity(a);
        } else {
            self.active_moves.insert(mv);
        }
    }

    /// Take a register from the freeze
    fn freeze(&mut self, code: &CodeSectionData) {
        let Some(reg) = self.freeze_worklist.pop() else {
            return;
        };
        self.simplify_worklist.insert(reg);
        self.freeze_moves(code, reg);
    }

    /// freeze the moves corresponding to the reg and place corresponding registers to those moves
    /// back into the simplify list
    fn freeze_moves(&mut self, code: &CodeSectionData, reg: VirtualReg) {
        for mv in self.related_moves(reg).collect::<Vec<_>>() {
            let (block, index) = mv;
            let Some((target, source)) = code.blocks[block].instructions[index].as_move() else {
                unimplemented!(
                    "{:?} is not a move but it is stored in the moves worklist",
                    code.blocks[block].instructions[index]
                );
            };
            let mv_reg = if self.get_alias(source) == self.get_alias(reg) {
                self.get_alias(target)
            } else {
                self.get_alias(source)
            };
            self.active_moves.swap_remove(&mv);
            self.frozen_moves.insert(mv);
            if !self.has_related_moves(mv_reg)
                && self.graph.pressure_of(mv_reg) < self.graph.capacity_of(mv_reg)
            {
                self.freeze_worklist.swap_remove(&mv_reg);
                self.simplify_worklist.insert(mv_reg);
            }
        }
    }

    /// select the register with the most pressure and move if to the spill worklist
    fn select_spill(&mut self, code: &CodeSectionData) {
        let Some(reg) = self
            .spill_worklist
            .iter()
            .max_by_key(|&&t| self.graph.pressure_of(t) - self.graph.capacity_of(t))
            .copied()
        else {
            return;
        };
        self.spill_worklist.swap_remove(&reg);
        self.simplify_worklist.insert(reg);
        self.freeze_moves(code, reg);
    }

    fn assign_colors(
        mut self,
        code: &CodeSectionData,
    ) -> Result<HashMap<VirtualReg, Color>, Box<AllocationFailure>> {
        let mut colors = HashMap::<VirtualReg, Color>::new();
        for reg in code
            .r8_registers
            .keys()
            .map(VirtualReg::R8)
            .chain(code.r16_registers.keys().map(VirtualReg::R16))
            .chain(code.zero_registers.keys().map(VirtualReg::Zero))
            .chain(code.carry_registers.keys().map(VirtualReg::Carry))
        {
            if let Some(color) = self.graph.precolor(reg) {
                colors.insert(reg, color);
            }
        }

        let mut reg_defs = IndexMap::<VirtualReg, Vec<&Instr>>::new();
        let mut reg_uses = IndexMap::<VirtualReg, Vec<&Instr>>::new();
        for block in code.blocks.values() {
            for instruction in block.instructions.iter() {
                for reg in instruction.register_defs() {
                    reg_defs.entry(reg).or_default().push(instruction);
                }
                for reg in instruction.register_uses() {
                    reg_uses.entry(reg).or_default().push(instruction);
                }
            }
        }

        while let Some(reg) = self.select_stack.pop_front() {
            let mut candidates = self.graph.permitted_colors(reg);
            for neighbor in self.graph.get_edges(reg) {
                let neighbor = self.get_alias(neighbor);
                if let Some(&color) = colors.get(&neighbor) {
                    candidates.retain(|candidate| !candidate.conflicts(color));
                }
            }
            match candidates.into_iter().next() {
                Some(color) => {
                    colors.insert(reg, color);
                    self.colored_nodes.insert(reg);
                    self.propagate_dependent_constraints(reg, color, &reg_defs, &reg_uses);
                }
                None => {
                    self.spilled_nodes.insert(reg);
                }
            }
        }
        for reg in self.coalesced_nodes.iter().copied() {
            let a = self.get_alias(reg);
            if let Some(&c) = colors.get(&a) {
                colors.insert(reg, c);
            }
        }
        if self.spilled_nodes.is_empty() {
            Ok(colors)
        } else {
            Err(Box::new(AllocationFailure {
                graph: self.graph,
                spills: self.spilled_nodes,
            }))
        }
    }

    fn has_related_moves(&self, reg: VirtualReg) -> bool {
        self.related_moves(reg).next().is_some()
    }

    fn related_moves(&self, reg: VirtualReg) -> impl Iterator<Item = (BlockId, usize)> {
        self.graph
            .moves(reg)
            .filter(|mv| self.active_moves.contains(mv) || self.worklist_moves.contains(mv))
    }

    fn get_edges_ignoring_processed(&self, reg: VirtualReg) -> IndexSet<VirtualReg> {
        self.graph
            .get_edges(reg)
            .filter(|other| !self.on_stack.contains(other) && !self.coalesced_nodes.contains(other))
            .collect::<IndexSet<_>>()
    }

    fn decrease_pressure(&mut self, reg: VirtualReg, delta: usize) {
        let before = self.graph.pressure_of(reg);
        let after = before.saturating_sub(delta);
        self.graph.set_pressure(reg, after);
        let cap = self.graph.capacity_of(reg);
        if before >= cap && after < cap {
            for adjacent in self.get_edges_ignoring_processed(reg) {
                self.enable_moves(adjacent);
            }
            self.enable_moves(reg);
            self.spill_worklist.swap_remove(&reg);
            if self.has_related_moves(reg) {
                self.freeze_worklist.insert(reg);
            } else {
                self.simplify_worklist.insert(reg);
            }
        }
    }

    fn enable_moves(&mut self, reg: VirtualReg) {
        for mv in self.related_moves(reg).collect::<Vec<_>>() {
            if self.active_moves.swap_remove(&mv) {
                self.worklist_moves.insert(mv);
            }
        }
    }

    fn get_alias(&self, mut reg: VirtualReg) -> VirtualReg {
        while self.coalesced_nodes.contains(&reg) {
            reg = self.alias[&reg];
        }
        reg
    }

    fn add_back_to_simplify_if_within_capacity(&mut self, reg: VirtualReg) {
        if !self.graph.is_precolored(reg)
            && !self.has_related_moves(reg)
            && self.graph.pressure_of(reg) < self.graph.capacity_of(reg)
        {
            self.freeze_worklist.swap_remove(&reg);
            self.simplify_worklist.insert(reg);
        }
    }

    fn george_test(&self, a: VirtualReg, b: VirtualReg, merged: &ColorSet) -> bool {
        self.graph.is_precolored(a)
            && !merged.is_empty()
            && self.get_edges_ignoring_processed(b).iter().all(|&t| {
                self.graph.pressure_of(t) < self.graph.capacity_of(t)
                    || self.graph.is_precolored(t)
                    || self.graph.has_edge(t, a)
            })
    }

    fn briggs_test(&self, a: VirtualReg, b: VirtualReg, merged: &ColorSet) -> bool {
        if self.graph.is_precolored(a) || merged.is_empty() {
            return false;
        }
        let mut neighbors = self.get_edges_ignoring_processed(a);
        neighbors.extend(self.get_edges_ignoring_processed(b));
        let bound = neighbors
            .iter()
            .filter(|&&neighbor| {
                self.graph.pressure_of(neighbor) >= self.graph.capacity_of(neighbor)
            })
            .map(|&neighbor| {
                self.graph
                    .permitted_colors(neighbor)
                    .into_iter()
                    .map(|c| {
                        merged
                            .clone()
                            .into_iter()
                            .filter(|&c2| c.conflicts(c2))
                            .count()
                    })
                    .max()
                    .unwrap_or(0)
            })
            .sum::<usize>();
        bound < merged.len()
    }

    fn combine(&mut self, a: VirtualReg, b: VirtualReg) {
        if self.freeze_worklist.contains(&b) {
            self.freeze_worklist.swap_remove(&b);
        } else {
            self.spill_worklist.swap_remove(&b);
        }
        self.coalesced_nodes.insert(b);
        self.alias.insert(b, a);

        let b_moves = self.graph.moves(b).collect::<IndexSet<_>>();
        self.graph.add_moves(a, b_moves);

        self.enable_moves(b);

        self.graph
            .constrain_colors_to(a, self.graph.permitted_colors(b));

        let mut pressure_a = 0usize;
        for neighbor in self.get_edges_ignoring_processed(a) {
            pressure_a = pressure_a.saturating_add(self.graph.blocked_colors_count(a, neighbor));
        }
        if !self.graph.is_precolored(a) {
            self.graph.set_pressure(a, pressure_a);
        }

        for neighbor in self.get_edges_ignoring_processed(b) {
            let delta = self.graph.retarget_edge(neighbor, b, a);
            self.decrease_pressure(neighbor, delta);
        }

        if self.graph.pressure_of(a) >= self.graph.capacity_of(a)
            && self.freeze_worklist.contains(&a)
        {
            self.freeze_worklist.swap_remove(&a);
            self.spill_worklist.insert(a);
        }
    }

    fn propagate_dependent_constraints(
        &mut self,
        reg: VirtualReg,
        color: Color,
        reg_defs: &IndexMap<VirtualReg, Vec<&Instr>>,
        reg_uses: &IndexMap<VirtualReg, Vec<&Instr>>,
    ) {
        for instruction in reg_defs
            .get(&reg)
            .map(|defs| defs.iter().copied())
            .into_iter()
            .flatten()
        {
            match instruction {
                Instr::Split { high, low, source } if VirtualReg::R8(*low) == reg => {
                    let (high_color, source_color) = match color {
                        Color::F => (Color::A, Color::AF),
                        Color::C => (Color::B, Color::BC),
                        Color::E => (Color::D, Color::DE),
                        Color::L => (Color::H, Color::HL),
                        unexpected => {
                            panic!("Bad coloring constraint. low cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*high), ColorSet::just(high_color));
                    self.graph.constrain_colors_to(
                        VirtualReg::R16(*source),
                        ColorSet::just(source_color),
                    );
                }
                Instr::Split { high, low, source } if VirtualReg::R8(*high) == reg => {
                    let (low_color, source_color) = match color {
                        Color::A => (Color::F, Color::AF),
                        Color::B => (Color::C, Color::BC),
                        Color::D => (Color::E, Color::DE),
                        Color::H => (Color::L, Color::HL),
                        unexpected => {
                            panic!("Bad coloring constraint. high cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*low), ColorSet::just(low_color));
                    self.graph.constrain_colors_to(
                        VirtualReg::R16(*source),
                        ColorSet::just(source_color),
                    );
                }
                Instr::Join {
                    high_in, low_in, ..
                } => {
                    let (high_color, low_color) = match color {
                        Color::AF => (Color::A, Color::F),
                        Color::BC => (Color::B, Color::C),
                        Color::DE => (Color::D, Color::E),
                        Color::HL => (Color::H, Color::L),
                        unexpected => {
                            panic!("Bad coloring constraint. 16-bit cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*low_in), ColorSet::just(low_color));
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*high_in), ColorSet::just(high_color));
                }
                _ => {}
            }
        }
        for instruction in reg_uses
            .get(&reg)
            .map(|defs| defs.iter().copied())
            .into_iter()
            .flatten()
        {
            match instruction {
                Instr::Join {
                    high_in,
                    low_in,
                    target,
                } if VirtualReg::R8(*low_in) == reg => {
                    let (high_color, target_color) = match color {
                        Color::C => (Color::B, Color::BC),
                        Color::E => (Color::D, Color::DE),
                        Color::L => (Color::H, Color::HL),
                        unexpected => {
                            panic!("Bad coloring constraint. low cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*high_in), ColorSet::just(high_color));
                    self.graph.constrain_colors_to(
                        VirtualReg::R16(*target),
                        ColorSet::just(target_color),
                    );
                }
                Instr::Join {
                    high_in,
                    low_in,
                    target,
                } if VirtualReg::R8(*high_in) == reg => {
                    let (low_color, target_color) = match color {
                        Color::B => (Color::C, Color::BC),
                        Color::D => (Color::E, Color::DE),
                        Color::H => (Color::L, Color::HL),
                        unexpected => {
                            panic!("Bad coloring constraint. high cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*low_in), ColorSet::just(low_color));
                    self.graph.constrain_colors_to(
                        VirtualReg::R16(*target),
                        ColorSet::just(target_color),
                    );
                }
                Instr::Split { high, low, .. } => {
                    let (high_color, low_color) = match color {
                        Color::BC => (Color::B, Color::C),
                        Color::DE => (Color::D, Color::E),
                        Color::HL => (Color::H, Color::L),
                        unexpected => {
                            panic!("Bad coloring constraint. 16-bit cannot be {unexpected:?}")
                        }
                    };
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*low), ColorSet::just(low_color));
                    self.graph
                        .constrain_colors_to(VirtualReg::R8(*high), ColorSet::just(high_color));
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct AllocationFailure {
    pub graph: InterferenceGraph,
    pub spills: IndexSet<VirtualReg>,
}

#[instrument(skip_all)]
fn rewrite_code(
    code: &mut CodeSectionData,
    spill_slots: &mut SpillSlots,
    mut failure: Box<AllocationFailure>,
) {
    tracing::debug!(spills = ?failure.spills, "rewrite_code");
    if cfg!(debug_assertions) {
        let precolored_spills = failure
            .spills
            .iter()
            .filter(|&&reg| match reg {
                VirtualReg::R8(reg) => code.r8_registers[reg].color.is_some(),
                VirtualReg::R16(reg) => code.r16_registers[reg].color.is_some(),
                VirtualReg::Zero(reg) => code.zero_registers[reg].color.is_some(),
                VirtualReg::Carry(reg) => code.carry_registers[reg].color.is_some(),
            })
            .collect::<Vec<_>>();
        debug_assert!(
            precolored_spills.is_empty(),
            "cannot spill a precolored register {precolored_spills:?}"
        );
    }

    let flag_spill_r8s = failure
        .spills
        .iter()
        .copied()
        .filter(|reg| matches!(reg, VirtualReg::Zero(_) | VirtualReg::Carry(_)))
        .map(|flag| (flag, code.r8_registers.insert(Register::default())))
        .collect::<IndexMap<_, _>>();
    failure
        .spills
        .retain(|reg| !flag_spill_r8s.contains_key(reg));

    let functions_blocks = calculate_function_blocks(code);
    allocate_spill_slots(
        code,
        spill_slots,
        &failure.spills,
        &failure.graph,
        &functions_blocks,
    );
    for block in code.blocks.keys().collect::<Vec<_>>() {
        rewrite_block_with_spills(code, block, spill_slots, &failure.spills, &flag_spill_r8s);
    }
    if !spill_slots.function_to_frame.is_empty() {
        rewrite_functions_with_frames(code, spill_slots, &functions_blocks);
    }
}

fn allocate_spill_slots(
    code: &mut CodeSectionData,
    spill_slots: &mut SpillSlots,
    new_spills: &IndexSet<VirtualReg>,
    graph: &InterferenceGraph,
    functions_blocks: &FunctionBlocks,
) {
    for reg in new_spills {
        match reg {
            VirtualReg::R8(reg) => {
                let function = code.r8_registers[*reg].defined.keys().fold(None, |result, block| {
                    let entry = functions_blocks.block_to_function.get(block).copied().unwrap_or_else(|| panic!("Cannot allocate spill for {reg:?} as it is defined in a block not in a function"));
                    if let Some(existing) = result {
                        assert_eq!(existing, entry, "Cannot allocate spill for {reg:?} as it belongs to multiple functions {existing} and {entry}");
                    }
                    Some(entry)
                }).expect("Cannot spill an r8 not defined anywhere");
                let frame = spill_slots
                    .function_to_frame
                    .entry(function)
                    .unwrap()
                    .or_default();
                let slot = frame.occupants.iter().position(|occupants| {
                    occupants
                        .iter()
                        .all(|occupant| !graph.has_edge(VirtualReg::R8(*reg), *occupant))
                });
                let slot = match slot {
                    Some(slot) => slot,
                    None => {
                        let slot = frame.occupants.len();
                        frame.occupants.push(Vec::new());
                        slot
                    }
                };
                frame.occupants[slot].push(VirtualReg::R8(*reg));
                spill_slots.r8_to_slot.insert(
                    *reg,
                    i8::try_from(slot).expect("More than 127 slots required"),
                );
            }
            VirtualReg::R16(reg) => {
                let function = code.r16_registers[*reg].defined.keys().fold(None, |result, block| {
                    let entry = functions_blocks.block_to_function.get(block).copied().unwrap_or_else(|| panic!("Cannot allocate spill for {reg:?} as it is defined in a block not in a function"));
                    if let Some(existing) = result {
                        assert_eq!(existing, entry, "Cannot allocate spill for {reg:?} as it belongs to multiple functions {existing} and {entry}");
                    }
                    Some(entry)
                }).expect("Cannot spill an r16 not defined anywhere");
                let frame = spill_slots
                    .function_to_frame
                    .entry(function)
                    .unwrap()
                    .or_default();
                let mut found_low = false;
                let slot = frame.occupants.iter().position(|occupants| {
                    let found = occupants
                        .iter()
                        .all(|occupant| !graph.has_edge(VirtualReg::R16(*reg), *occupant));
                    if found {
                        if found_low {
                            true
                        } else {
                            found_low = true;
                            false
                        }
                    } else {
                        false
                    }
                });
                let slot = match slot {
                    Some(slot) => slot - 1,
                    None => {
                        if !found_low {
                            frame.occupants.push(Vec::new());
                        }
                        let slot = frame.occupants.len();
                        frame.occupants.push(Vec::new());
                        slot - 1
                    }
                };
                frame.occupants[slot].push(VirtualReg::R16(*reg));
                spill_slots.r16_to_slot.insert(
                    *reg,
                    i8::try_from(slot).expect("More than 127 slots required"),
                );
            }
            VirtualReg::Zero(_) | VirtualReg::Carry(_) => {
                unimplemented!("flag spills are handled differently")
            }
        }
    }
}

fn rewrite_block_with_spills(
    code: &mut CodeSectionData,
    block_id: BlockId,
    spill_slots: &SpillSlots,
    spills: &IndexSet<VirtualReg>,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
) {
    let old_instructions = std::mem::take(&mut code.blocks[block_id].instructions);
    let mut new_instructions = Vec::with_capacity(old_instructions.len());

    for instruction in old_instructions {
        let mut use_substitutions = HashMap::<VirtualReg, VirtualReg>::new();

        for reg in instruction.register_uses() {
            if spills.contains(&reg) && !use_substitutions.contains_key(&reg) {
                let fresh = load_spill_slot(
                    code,
                    block_id,
                    &mut new_instructions,
                    reg,
                    spill_slots,
                    flag_spill_r8s,
                );
                use_substitutions.insert(reg, fresh);
            }
        }

        let defs_requiring_stores = instruction
            .register_defs()
            .filter(|reg| spills.contains(reg))
            .collect::<Vec<_>>();
        let defs = instruction.register_defs().collect::<Vec<_>>();

        let index = new_instructions.len();
        new_instructions.push(rewrite_instruction(instruction, &use_substitutions));
        // as we are inserting new instructions, we need to update the defined index to match
        for reg in defs {
            match reg {
                VirtualReg::R8(reg) => {
                    code.r8_registers[reg].defined.insert(block_id, index);
                }
                VirtualReg::R16(reg) => {
                    code.r16_registers[reg].defined.insert(block_id, index);
                }
                VirtualReg::Zero(reg) => {
                    code.zero_registers[reg].defined.insert(block_id, index);
                }
                VirtualReg::Carry(reg) => {
                    code.carry_registers[reg].defined.insert(block_id, index);
                }
            }
        }

        for reg in defs_requiring_stores {
            store_spill_slot(
                code,
                block_id,
                &mut new_instructions,
                reg,
                spill_slots,
                flag_spill_r8s,
            );
        }
    }

    code.blocks[block_id].instructions = new_instructions;
}

fn load_spill_slot(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    reg: VirtualReg,
    spill_slots: &SpillSlots,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
) -> VirtualReg {
    match reg {
        VirtualReg::R8(r8) => {
            let slot = spill_slots.r8_to_slot[r8];
            let source_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::LdSPE8 {
                target: source_addr,
                c,
                right: slot,
            });
            let target = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Ld8Ptr {
                target,
                source_addr: Addr::Reg(source_addr.upcast_to_r16_without_move()),
            });
            VirtualReg::R8(target)
        }
        VirtualReg::R16(r16) => {
            let slot = spill_slots.r16_to_slot[r16];
            let source_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::LdSPE8 {
                target: source_addr,
                c,
                right: slot,
            });
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let revised_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            instructions.push(Instr::Ld8Inc {
                target: a,
                revised_addr,
                source_addr,
            });
            let low_in = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Ld8 {
                target: low_in,
                source: a.upcast_to_r8_without_move(),
            });
            let high_in = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Ld8Ptr {
                target: high_in,
                source_addr: Addr::Reg(revised_addr.upcast_to_r16_without_move()),
            });
            let target = code.r16_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Join {
                target,
                low_in,
                high_in,
            });
            VirtualReg::R16(target)
        }
        VirtualReg::Zero(flag) => {
            let source = flag_spill_r8s[&VirtualReg::Zero(flag)];
            let one = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            instructions.push(Instr::Ld8Value {
                target: one.upcast_to_r8_without_move(),
                source: 0xff,
            });
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let target = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            // zero flag is stored as 0xff when set and 0x00 when not set. To set it back to the
            // zero flag we need to take the complement of the value, but CPL doesn't set the zero
            // flag so XOR is the next best instruction.
            instructions.push(Instr::Xor {
                target,
                z,
                c,
                left: one,
                right: Operand8::Reg(source),
            });
            VirtualReg::Zero(z)
        }
        VirtualReg::Carry(flag) => {
            let source = flag_spill_r8s[&VirtualReg::Carry(flag)];
            // even though the register will contain 0xFF if true and 0x00 if false
            // and therefore a rotate instruction will not change the value, we still
            // need to do a move to not confuse the register allocator and the defined
            // declarations. To avoid this we would need to change how the LIR instructions
            // are defined
            let moved = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Ld8 {
                target: moved,
                source,
            });
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let target = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Rrc {
                target,
                z,
                c,
                source: moved,
            });
            VirtualReg::Carry(c)
        }
    }
}

fn store_spill_slot(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    reg: VirtualReg,
    spill_slots: &SpillSlots,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
) {
    match reg {
        VirtualReg::R8(r8) => {
            let slot = spill_slots.r8_to_slot[r8];
            let target_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::LdSPE8 {
                target: target_addr,
                c,
                right: slot,
            });
            instructions.push(Instr::Store {
                target_addr: Addr::Reg(target_addr.upcast_to_r16_without_move()),
                source: r8,
            });
        }
        VirtualReg::R16(r16) => {
            let slot = spill_slots.r16_to_slot[r16];
            let target_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::LdSPE8 {
                target: target_addr,
                c,
                right: slot,
            });
            let low = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let high = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Split {
                high,
                low,
                source: r16,
            });
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            // Currently uses `LD A, R8` then `LD [HLI], A` instead of
            // `LD HL, SP+e8` for the low byte and `LD HL, SP+e8+1` for the high
            // byte as it is less cycle time at the cost of interfering with `A`.
            // Given most instructions which aren't loads interfere with `A` this
            // seems like a reasonable tradeoff
            instructions.push(Instr::Ld8 {
                target: a.upcast_to_r8_without_move(),
                source: low,
            });
            let revised_addr = code
                .r16_registers
                .insert(Register {
                    color: Some(Color::HL),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_hl_without_move();
            instructions.push(Instr::StoreInc {
                revised_addr,
                addr: target_addr,
                source: a,
            });
            instructions.push(Instr::Store {
                target_addr: Addr::Reg(revised_addr.upcast_to_r16_without_move()),
                source: high,
            });
        }
        VirtualReg::Zero(z_in) => {
            // zero flags are difficult, unfortunately. We need to push them onto
            // the stack then pop onto a 16-bit register
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let c_in = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::PushAF {
                source: a,
                z_in,
                c_in,
            });
            let target = code.r16_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::PopR16 { target });
            let low = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let high = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Split {
                high,
                low,
                source: target,
            });
            // low now has the zero flag stored in the 7th bit. We can now
            // use the rotate left strategy
            let target = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c_in = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Rlc {
                target,
                z,
                c: c_in,
                source: low,
            });
            let a1 = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let a2 = code.r8_registers.insert(Register {
                color: Some(Color::A),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let source = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Sbc {
                target: source,
                z,
                c,
                left: a1,
                right: Operand8::Reg(a2),
                c_in,
            });
            let target = flag_spill_r8s[&VirtualReg::Zero(z_in)];
            code.r8_registers[target]
                .defined
                .insert(block_id, instructions.len());
            instructions.push(Instr::Ld8 {
                target,
                source: source.upcast_to_r8_without_move(),
            });
        }
        VirtualReg::Carry(c_in) => {
            let a1 = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let a2 = code.r8_registers.insert(Register {
                color: Some(Color::A),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let source = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Sbc {
                target: source,
                z,
                c,
                left: a1,
                right: Operand8::Reg(a2),
                c_in,
            });
            let target = flag_spill_r8s[&VirtualReg::Carry(c_in)];
            code.r8_registers[target]
                .defined
                .insert(block_id, instructions.len());
            instructions.push(Instr::Ld8 {
                target,
                source: source.upcast_to_r8_without_move(),
            });
        }
    }
}

fn rewrite_instruction(
    mut instruction: Instr,
    use_substitutions: &HashMap<VirtualReg, VirtualReg>,
) -> Instr {
    if use_substitutions.is_empty() {
        return instruction;
    }
    let sub_r8 = |r8: R8Id| {
        use_substitutions
            .get(&VirtualReg::R8(r8))
            .map(|sub| {
                let VirtualReg::R8(r8) = sub else {
                    panic!("{r8:?} substitutes to non-r8 {sub:?}")
                };
                *r8
            })
            .unwrap_or(r8)
    };
    let sub_a = |a: AId| sub_r8(a.upcast_to_r8_without_move()).downcast_to_a_without_move();
    let sub_r16 = |r16: R16Id| {
        use_substitutions
            .get(&VirtualReg::R16(r16))
            .map(|sub| {
                let VirtualReg::R16(r16) = sub else {
                    panic!("{r16:?} substitutes to non-r16 {sub:?}")
                };
                *r16
            })
            .unwrap_or(r16)
    };
    let sub_hl = |hl: HLId| sub_r16(hl.upcast_to_r16_without_move()).downcast_to_hl_without_move();
    let sub_zero = |z: ZeroId| {
        use_substitutions
            .get(&VirtualReg::Zero(z))
            .map(|sub| {
                let VirtualReg::Zero(z) = sub else {
                    panic!("{z:?} substitutes to non-zero {sub:?}")
                };
                *z
            })
            .unwrap_or(z)
    };
    let sub_carry = |c: CarryId| {
        use_substitutions
            .get(&VirtualReg::Carry(c))
            .map(|sub| {
                let VirtualReg::Carry(c) = sub else {
                    panic!("{c:?} substitutes to non-carry {sub:?}")
                };
                *c
            })
            .unwrap_or(c)
    };
    match &mut instruction {
        Instr::Nop
        | Instr::Ld8Value { .. }
        | Instr::Ld16 { .. }
        | Instr::Scf { .. }
        | Instr::AddSP { .. }
        | Instr::LdSPE8 { .. }
        | Instr::PopR16 { .. }
        | Instr::PopAF { .. }
        | Instr::LdReturn8 { .. }
        | Instr::LdReturn16 { .. } => {}
        Instr::Ld8 { source, .. }
        | Instr::Dec { source, .. }
        | Instr::Inc { source, .. }
        | Instr::Bit { source, .. }
        | Instr::Res { source, .. }
        | Instr::Set { source, .. }
        | Instr::Rlc { source, .. }
        | Instr::Rrc { source, .. }
        | Instr::Sla { source, .. }
        | Instr::Sra { source, .. }
        | Instr::Srl { source, .. }
        | Instr::Swap { source, .. } => {
            *source = sub_r8(*source);
        }
        Instr::Cpl { source, .. } => {
            *source = sub_a(*source);
        }
        Instr::Ld8Ptr { source_addr, .. } => {
            if let Addr::Reg(source_addr) = source_addr {
                *source_addr = sub_r16(*source_addr);
            }
        }
        Instr::Ld8HighPtr { source_addr, .. } => {
            if let HighAddr::Reg(source_addr) = source_addr {
                *source_addr = sub_r8(*source_addr);
            }
        }
        Instr::Ld8Inc {
            source_addr: addr, ..
        }
        | Instr::Ld8Dec {
            source_addr: addr, ..
        }
        | Instr::StoreValue { addr, .. }
        | Instr::DecPtr { addr, .. }
        | Instr::IncPtr { addr, .. }
        | Instr::BitPtr {
            source_addr: addr, ..
        }
        | Instr::ResPtr { addr, .. }
        | Instr::SetPtr { addr, .. }
        | Instr::RlcPtr { addr, .. }
        | Instr::RrcPtr { addr, .. }
        | Instr::SlaPtr { addr, .. }
        | Instr::SraPtr { addr, .. }
        | Instr::SrlPtr { addr, .. }
        | Instr::SwapPtr { addr, .. } => {
            *addr = sub_hl(*addr);
        }
        Instr::Store {
            target_addr,
            source,
        } => {
            if let Addr::Reg(target_addr) = target_addr {
                *target_addr = sub_r16(*target_addr);
            }
            *source = sub_r8(*source);
        }
        Instr::StoreHigh {
            addr: target_addr,
            source,
        } => {
            if let HighAddr::Reg(target_addr) = target_addr {
                *target_addr = sub_r8(*target_addr);
            }
            *source = sub_a(*source);
        }
        Instr::StoreInc {
            addr: target_addr,
            source,
            ..
        }
        | Instr::StoreDec {
            addr: target_addr,
            source,
            ..
        } => {
            *target_addr = sub_hl(*target_addr);
            *source = sub_a(*source);
        }
        Instr::Adc {
            left, right, c_in, ..
        }
        | Instr::Sbc {
            left, right, c_in, ..
        } => {
            *left = sub_a(*left);
            if let Operand8::Reg(right) = right {
                *right = sub_r8(*right);
            }
            *c_in = sub_carry(*c_in);
        }
        Instr::Add { left, right, .. }
        | Instr::Cp { left, right, .. }
        | Instr::Sub { left, right, .. }
        | Instr::And { left, right, .. }
        | Instr::Or { left, right, .. }
        | Instr::Xor { left, right, .. } => {
            *left = sub_a(*left);
            if let Operand8::Reg(right) = right {
                *right = sub_r8(*right);
            }
        }
        Instr::Rl { source, c_in, .. } | Instr::Rr { source, c_in, .. } => {
            *source = sub_r8(*source);
            *c_in = sub_carry(*c_in);
        }
        Instr::RlPtr { addr, c_in, .. } | Instr::RrPtr { addr, c_in, .. } => {
            *addr = sub_hl(*addr);
            *c_in = sub_carry(*c_in);
        }
        Instr::Ccf { c_in, .. } | Instr::MoveCarry { c_in, .. } => {
            *c_in = sub_carry(*c_in);
        }
        Instr::Add16 { left, right, .. } => {
            *left = sub_hl(*left);
            *right = sub_r16(*right);
        }
        Instr::Dec16 { source, .. }
        | Instr::Inc16 { source, .. }
        | Instr::PushR16 { source }
        | Instr::Split { source, .. } => {
            *source = sub_r16(*source);
        }
        Instr::PushAF { source, z_in, c_in } => {
            *source = sub_a(*source);
            *z_in = sub_zero(*z_in);
            *c_in = sub_carry(*c_in);
        }
        Instr::Join {
            high_in, low_in, ..
        } => {
            *low_in = sub_r8(*low_in);
            *high_in = sub_r8(*high_in);
        }
        Instr::MoveZero { z_in, .. } => {
            *z_in = sub_zero(*z_in);
        }
    }
    instruction
}

fn rewrite_functions_with_frames(
    code: &mut CodeSectionData,
    spill_slots: &SpillSlots,
    functions_blocks: &FunctionBlocks,
) {
    let mut functions = spill_slots.function_to_frame.keys().collect::<Vec<_>>();
    // we want to ensure that we process the frames in a known order so that any incidental
    // blocks or registers allocated have a stable value
    functions.sort();
    for entry in functions {
        let frame = &spill_slots.function_to_frame[entry];
        rewrite_function_entry_with_frame(code, entry, frame);
        rewrite_function_exits_with_frame(code, &functions_blocks.function_to_exits[entry], frame);
    }
}

fn rewrite_function_entry_with_frame(code: &mut CodeSectionData, entry: BlockId, frame: &Frame) {
    if !matches!(
        code.blocks[entry].instructions.first(),
        Some(Instr::AddSP { .. })
    ) {
        let z = code.zero_registers.insert(Register {
            color: Some(Color::Fz),
            defined: [(entry, 0)].into_iter().collect(),
        });
        let c = code.carry_registers.insert(Register {
            color: Some(Color::Fc),
            defined: [(entry, 0)].into_iter().collect(),
        });
        code.blocks[entry]
            .instructions
            .insert(0, Instr::AddSP { z, c, right: 0 });
    }
    if let Some(Instr::AddSP { right, .. }) = &mut code.blocks[entry].instructions.first_mut() {
        *right = -i8::try_from(frame.occupants.len()).expect("More than 127 slots required");
    } else {
        unreachable!("first instruction of block {entry} should be AddSP");
    }
}

fn rewrite_function_exits_with_frame(
    code: &mut CodeSectionData,
    exits: &StableSparseSlotKeySet<BlockId>,
    frame: &Frame,
) {
    for mut exit in exits {
        if let Terminator::RetIf {
            source,
            negate,
            else_block,
        } = &code.blocks[exit].terminator
        {
            let source = *source;
            let negate = *negate;
            let else_block = *else_block;
            let then_block = code.blocks.insert(Block {
                instructions: Vec::new(),
                terminator: Terminator::Ret,
                previous: vec![exit],
            });
            code.blocks[exit].terminator = Terminator::JpIf {
                source,
                negate,
                then_block,
                else_block,
            };
            exit = then_block;
        }
        if !matches!(
            code.blocks[exit].instructions.last(),
            Some(Instr::AddSP { .. })
        ) {
            let index = code.blocks[exit].instructions.len();
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(exit, index)].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(exit, index)].into_iter().collect(),
            });
            code.blocks[exit]
                .instructions
                .push(Instr::AddSP { z, c, right: 0 });
        }
        if let Some(Instr::AddSP { right, .. }) = &mut code.blocks[exit].instructions.last_mut() {
            *right = i8::try_from(frame.occupants.len()).expect("More than 127 slots required");
        } else {
            unreachable!("last instruction of block {exit} should be AddSP");
        }
    }
}

fn cleanup_redundant_instructions(code: &mut CodeSectionData) {
    for (block_id, block) in code.blocks.iter_mut() {
        for instruction in std::mem::take(&mut block.instructions) {
            let retain = match &instruction {
                Instr::Ld8 { target, source }
                    if code.r8_registers[*target].color == code.r8_registers[*source].color =>
                {
                    false
                }
                Instr::Nop
                | Instr::Join { .. }
                | Instr::Split { .. }
                | Instr::MoveZero { .. }
                | Instr::MoveCarry { .. }
                | Instr::LdReturn8 { .. }
                | Instr::LdReturn16 { .. } => false,
                _ => true,
            };
            if retain {
                let index = block.instructions.len();
                for reg in instruction.register_defs() {
                    match reg {
                        VirtualReg::R8(reg) => {
                            code.r8_registers[reg].defined.insert(block_id, index)
                        }
                        VirtualReg::R16(reg) => {
                            code.r16_registers[reg].defined.insert(block_id, index)
                        }
                        VirtualReg::Zero(reg) => {
                            code.zero_registers[reg].defined.insert(block_id, index)
                        }
                        VirtualReg::Carry(reg) => {
                            code.carry_registers[reg].defined.insert(block_id, index)
                        }
                    };
                }
                block.instructions.push(instruction);
            } else {
                for reg in instruction.register_defs() {
                    // we don't want to remove the register as we are not updating the
                    // uses of the register. Instead, we just clean up the definition pointer
                    match reg {
                        VirtualReg::R8(reg) => code.r8_registers[reg].defined.remove(block_id),
                        VirtualReg::R16(reg) => code.r16_registers[reg].defined.remove(block_id),
                        VirtualReg::Zero(reg) => code.zero_registers[reg].defined.remove(block_id),
                        VirtualReg::Carry(reg) => {
                            code.carry_registers[reg].defined.remove(block_id)
                        }
                    };
                }
            }
        }
    }
}

#[derive(Default)]
struct Frame {
    occupants: Vec<Vec<VirtualReg>>,
}

#[derive(Default)]
struct SpillSlots {
    function_to_frame: SparseSecondaryMap<BlockId, Frame>,
    r8_to_slot: SparseSecondaryMap<R8Id, i8>,
    r16_to_slot: SparseSecondaryMap<R16Id, i8>,
}

#[derive(Default)]
struct FunctionBlocks {
    block_to_function: SparseSecondaryMap<BlockId, BlockId>,
    function_to_exits: SparseSecondaryMap<BlockId, StableSparseSlotKeySet<BlockId>>,
}
