use crate::codegen::interference::{
    BlockLiveness, InterferenceGraph, build_interference_graph, calculate_block_liveness,
};
use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::lir::{
    AId, Addr, Block, BlockId, CarryId, CodeSectionData, FlagId, HLId, HighAddr, Instr, Lir,
    Operand8, R8Id, R16Id, Register, SectionData, Terminator, VirtualReg, ZeroId,
};
use crate::reg::{Color, ColorSet};
use indexmap::{IndexMap, IndexSet};
use quokka_lang_slotkeyset::{SparseSlotKeySet, StableSparseSlotKeySet};
use slotmap::SparseSecondaryMap;
use std::collections::{HashMap, VecDeque};
use tracing::instrument;

/// Drives register allocation for a whole [`Lir`], across all of its code sections.
#[derive(Default)]
pub struct RegAllocator {
    features: PreviewFeatureSet,
}

impl RegAllocator {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    #[instrument(skip_all)]
    pub fn allocate(&self, lir: &mut Lir) {
        for section in lir.sections.values_mut() {
            if let SectionData::Code(code) = &mut section.data {
                // Sections with no virtual registers are hand-emitted final machine
                // code (e.g. the cartridge header stub pinned at `$0100`). There is
                // nothing to allocate.
                if !code.has_registers() {
                    continue;
                }
                let colors = self.allocate_registers_for_code(code);
                for (reg, color) in colors {
                    code.register_data_mut(reg).color = Some(color);
                }
            }
        }
    }

    #[instrument(skip_all)]
    fn allocate_registers_for_code(
        &self,
        code: &mut CodeSectionData,
    ) -> HashMap<VirtualReg, Color> {
        let mut spill_slots = SpillSlots::default();
        let max_iterations = code.r8_registers.len()
            + code.r16_registers.len()
            + code.zero_registers.len()
            + code.carry_registers.len()
            + 100;
        let mut previous_spills = IndexSet::new();
        for i in 0..max_iterations {
            match self.allocate_registers_round(i, code, &mut spill_slots) {
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
        &self,
        i: usize,
        code: &mut CodeSectionData,
        spill_slots: &mut SpillSlots,
    ) -> Result<HashMap<VirtualReg, Color>, IndexSet<VirtualReg>> {
        let block_liveness = calculate_block_liveness(code);
        match self.try_allocate_registers(code, &block_liveness, &spill_slots.short_lived) {
            Ok(colors) => Ok(colors),
            Err(failure) => {
                let spills = failure.spills.clone();
                rewrite_code(code, &block_liveness, spill_slots, failure);
                Err(spills)
            }
        }
    }

    #[instrument(skip_all)]
    fn try_allocate_registers(
        &self,
        code: &CodeSectionData,
        block_liveness: &BlockLiveness,
        short_lived: &IndexSet<VirtualReg>,
    ) -> Result<HashMap<VirtualReg, Color>, Box<AllocationFailure>> {
        let (graph, moves) = build_interference_graph(code, block_liveness);
        let allocator = Colorer {
            graph,
            worklist_moves: moves,
            short_lived: short_lived.clone(),
            ..Default::default()
        };
        allocator.run(code)
    }
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
                Terminator::Ret | Terminator::RetIf { .. } | Terminator::Reti
            ) {
                exits.insert(block);
            }
        }
        result.function_to_exits.insert(entry, exits);
    }
    result
}

/// Performs iterated register coalescing (George/Appel) graph coloring for a single round over
/// a single [`CodeSectionData`]. It only decides colors (and which registers must spill); it
/// does not rewrite code — spilled registers are rewritten by [`rewrite_code`], and the whole
/// process is retried in a fresh round until allocation succeeds, all driven by [`RegAllocator`].
#[derive(Default)]
struct Colorer {
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

    /// registers which earlier rounds have already cut down to the shortest range they can
    /// have, see [`SpillSlots::short_lived`]
    short_lived: IndexSet<VirtualReg>,
}

impl Colorer {
    fn run(
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
        self.assign_colors()
    }

    fn initialize_work(&mut self, code: &CodeSectionData) {
        for reg in code.registers() {
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

        if !merged.is_empty() && (self.george_test(a, b) || self.briggs_test(a, b, &merged)) {
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

    fn assign_colors(mut self) -> Result<HashMap<VirtualReg, Color>, Box<AllocationFailure>> {
        let mut colors = HashMap::<VirtualReg, Color>::new();
        // copy across precolored registers first. Use the graph nodes which includes
        // synthetic nodes such as `VirtualReg::Call` as well
        for (reg, color) in self.graph.precolored() {
            colors.insert(reg, color);
        }

        while let Some(reg) = self.select_stack.pop_front() {
            let mut candidates = self.graph.permitted_colors(reg);
            for neighbor in self.graph.get_edges(reg) {
                let neighbor = self.get_alias(neighbor);
                if let Some(&color) = colors.get(&neighbor) {
                    candidates.retain(|candidate| !candidate.conflicts(color));
                }
            }

            // Registers still waiting to be colored which this one interferes with. Committing
            // to a color removes it (and anything overlapping it, such as `B` removing `BC`)
            // from their permitted sets, so a candidate which empties one of them is a dead
            // end: prefer any other candidate before falling back to it.
            let uncolored_neighbors = self
                .graph
                .get_edges(reg)
                .map(|neighbor| self.get_alias(neighbor))
                .filter(|neighbor| *neighbor != reg && !colors.contains_key(neighbor))
                .collect::<IndexSet<_>>();

            let mut selected = None;
            for color in candidates {
                // A register tied to this one by a constraint (the halves and the pair of a
                // `Join`/`Split`) does not interfere with it, so the colors taken by the
                // registers it does interfere with have to be accounted for here. Otherwise the
                // tied register is left with nothing and spills instead of this one, and it may
                // be a reload of an earlier spill, which spilling again does nothing for.
                let propagated_permitted_colors = self.graph.calculate_propagated_permitted_colors(
                    reg,
                    ColorSet::just(color),
                    |tied, mut permitted| {
                        let tied = self.get_alias(tied);
                        if tied == reg || colors.contains_key(&tied) {
                            return permitted;
                        }
                        for other in self.graph.get_edges(tied) {
                            if let Some(&taken) = colors.get(&self.get_alias(other)) {
                                permitted.retain(|candidate| !candidate.conflicts(taken));
                            }
                        }
                        permitted
                    },
                );
                if propagated_permitted_colors
                    .values()
                    .any(|set| set.is_empty())
                {
                    // the candidate will result in another register becoming uncolorable due to
                    // constraint propagation so we will need to try and different one
                    tracing::trace!(
                        reg = ?reg,
                        color = ?color,
                        propagated_permitted_colors = ?propagated_permitted_colors,
                        "Candidate color violates constraints"
                    );
                    continue;
                }
                let starved = uncolored_neighbors.iter().copied().find(|&neighbor| {
                    let mut permitted = propagated_permitted_colors
                        .get(&neighbor)
                        .cloned()
                        .unwrap_or_else(|| self.graph.permitted_colors(neighbor));
                    permitted.retain(|candidate| !candidate.conflicts(color));
                    for other in self.graph.get_edges(neighbor) {
                        let other = self.get_alias(other);
                        if let Some(&taken) = colors.get(&other) {
                            permitted.retain(|candidate| !candidate.conflicts(taken));
                        }
                    }
                    permitted.is_empty()
                });
                if let Some(starved) = starved {
                    // taking this color leaves an interfering register with nothing to take.
                    // this is what fragments the register file for the wider `r16` pairs, e.g.
                    // coloring two unrelated `r8`s `B` and `D` leaves no aligned pair free.
                    tracing::trace!(
                        reg = ?reg,
                        color = ?color,
                        starved = ?starved,
                        "Candidate color starves an uncolored neighbor"
                    );
                    selected = selected.or(Some(color));
                    continue;
                }
                selected = Some(color);
                break;
            }
            let Some(color) = selected else {
                // Spilling a register which is already as short as it gets changes nothing, and
                // the next round would fail on it in just the same way. What it needs is for a
                // register holding one of its colors over a longer range to give it up.
                let evicted = if self.short_lived.contains(&reg) {
                    self.select_evictions(reg, &colors)
                } else {
                    None
                };
                match evicted {
                    Some(evicted) => {
                        tracing::trace!(
                            reg = ?reg,
                            evicted = ?evicted,
                            "Spilling the registers in the way of a short-lived register"
                        );
                        self.spilled_nodes.extend(evicted);
                    }
                    None => {
                        self.spilled_nodes.insert(reg);
                    }
                }
                continue;
            };
            colors.insert(reg, color);
            self.colored_nodes.insert(reg);
            // this commits the propagated permitted colors. Unfortunately we will
            // need to do the calculation twice
            self.graph.constrain_colors_to(reg, ColorSet::just(color));
        }
        for reg in self.coalesced_nodes.iter().copied().collect::<Vec<_>>() {
            let a = self.get_alias(reg);
            if let Some(&c) = colors.get(&a) {
                colors.insert(reg, c);
            }
        }
        if self.spilled_nodes.is_empty() {
            // Synthetic nodes (currently just `VirtualReg::Call`) exist only to constrain real
            // registers' colors above; they have no `CodeSectionData` storage to write a color
            // back into, so they must not leak into the result the caller writes back with.
            colors.retain(|reg, _| !reg.is_synthetic());
            Ok(colors)
        } else {
            Err(Box::new(AllocationFailure {
                graph: self.graph,
                spills: self.spilled_nodes,
            }))
        }
    }

    /// Chooses the registers to spill so that `reg`, which could not be colored, can take one of
    /// its permitted colors in a later round: the colored registers it interferes with which hold
    /// that color. The color needing the fewest registers spilled is chosen. A color held by a
    /// precolored register can never be freed, and one held only by short-lived registers has
    /// nothing worth spilling, so `None` is returned if every color is one of those.
    fn select_evictions(
        &self,
        reg: VirtualReg,
        colors: &HashMap<VirtualReg, Color>,
    ) -> Option<IndexSet<VirtualReg>> {
        let mut selected: Option<IndexSet<VirtualReg>> = None;
        'candidates: for candidate in self.graph.permitted_colors(reg) {
            let mut evicted = IndexSet::new();
            for neighbor in self.graph.get_edges(reg) {
                let alias = self.get_alias(neighbor);
                let Some(&taken) = colors.get(&alias) else {
                    continue;
                };
                if !candidate.conflicts(taken) {
                    continue;
                }
                if self.graph.is_precolored(alias) {
                    continue 'candidates;
                }
                if !self.short_lived.contains(&neighbor) {
                    evicted.insert(neighbor);
                }
            }
            if !evicted.is_empty()
                && selected
                    .as_ref()
                    .is_none_or(|selected| evicted.len() < selected.len())
            {
                selected = Some(evicted);
            }
        }
        selected
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

    fn george_test(&self, a: VirtualReg, b: VirtualReg) -> bool {
        let Some(color) = self.graph.precolor(a) else {
            return false;
        };
        self.get_edges_ignoring_processed(b).iter().all(|&t| {
            self.graph.pressure_of(t) < self.graph.capacity_of(t)
                || self.is_precolored_without_conflict(t, color)
                || self.graph.has_edge(t, a)
        })
    }

    fn is_precolored_without_conflict(&self, t: VirtualReg, color: Color) -> bool {
        self.graph
            .precolor(self.get_alias(t))
            .map(|t_color| !t_color.conflicts(color))
            .unwrap_or(false)
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

    /// combine `b` an into `a`, constraining `a` to the permitted colors of `b` and adding
    /// all `b`'s dependent nodes to `a` (including the inverse
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
        if let Some(constraints) = self.graph.constraints(b).cloned() {
            for (other, kind) in constraints {
                self.graph.add_constraint(a, other, kind);
            }
        }

        let pressure_a_before = self.graph.pressure_of(a);
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

        let cap_a = self.graph.capacity_of(a);
        if pressure_a_before >= cap_a && pressure_a < cap_a && self.spill_worklist.contains(&a) {
            for adjacent in self.get_edges_ignoring_processed(a) {
                self.enable_moves(adjacent);
            }
            self.enable_moves(a);
            self.spill_worklist.swap_remove(&a);
            if self.has_related_moves(a) {
                self.freeze_worklist.insert(a);
            } else {
                self.simplify_worklist.insert(a);
            }
        } else if self.graph.pressure_of(a) >= cap_a && self.freeze_worklist.contains(&a) {
            self.freeze_worklist.swap_remove(&a);
            self.spill_worklist.insert(a);
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
    block_liveness: &BlockLiveness,
    spill_slots: &mut SpillSlots,
    failure: Box<AllocationFailure>,
) {
    tracing::debug!(spills = ?failure.spills, "rewrite_code");
    if cfg!(debug_assertions) {
        let precolored_spills = failure
            .spills
            .iter()
            .filter(|&&reg| code.register_data(reg).color.is_some())
            .collect::<Vec<_>>();
        debug_assert!(
            precolored_spills.is_empty(),
            "cannot spill a precolored register {precolored_spills:?}"
        );
    }

    let registers_before = code.registers().collect::<IndexSet<_>>();
    let flag_spill_r8s = failure
        .spills
        .iter()
        .copied()
        .filter(|reg| matches!(reg, VirtualReg::Zero(_) | VirtualReg::Carry(_)))
        .map(|flag| (flag, code.r8_registers.insert(Register::default())))
        .collect::<IndexMap<_, _>>();
    let mut non_flag_spills = failure.spills.clone();
    non_flag_spills.retain(|reg| !flag_spill_r8s.contains_key(reg));

    let functions_blocks = calculate_function_blocks(code);
    allocate_spill_slots(
        code,
        spill_slots,
        &non_flag_spills,
        &failure.graph,
        &functions_blocks,
    );
    // Liveness of the code as it exists *before* any spill-rewrite instructions are inserted
    // below. `rewrite_block_with_spills` uses this to notice when a spill-slot address
    // computation (`LD HL, SP+e8`, which always clobbers `HL` and the flags) would land in the
    // middle of some other already-precolored `HL`/`A`/`Fz`/`Fc` register's live range, and
    // bracket the computation with `PUSH`/`POP` to protect it
    for block in code.blocks.keys().collect::<Vec<_>>() {
        let live_at =
            liveness_before_each_instruction(&code.blocks[block], &block_liveness.live_out[block]);
        rewrite_block_with_spills(
            code,
            block,
            spill_slots,
            &failure.spills,
            &flag_spill_r8s,
            &live_at,
        );
    }
    if !spill_slots.function_to_frame.is_empty() {
        rewrite_functions_with_frames(code, spill_slots, &functions_blocks);
    }
    // A spilled register is now only live from each definition to the store after it, and the
    // registers created to reload it only up to the instruction they were reloaded for. The `r8`
    // a spilled flag is kept in is the exception: it takes on the whole of the flag's range.
    spill_slots
        .short_lived
        .extend(failure.spills.iter().copied());
    spill_slots.short_lived.extend(code.registers().filter(|reg| {
        !registers_before.contains(reg)
            && !matches!(reg, VirtualReg::R8(r8) if flag_spill_r8s.values().any(|flag_r8| flag_r8 == r8))
    }));
    code.check_integrity().unwrap();
}

/// Computes, for each instruction index in `block` (plus one extra trailing entry for the point
/// just before the terminator), the set of registers live at that exact program point - i.e.
/// defined earlier and still needed later. Seeded from `live_out`, the block's live-out set as
/// computed by [`calculate_block_liveness`].
fn liveness_before_each_instruction(
    block: &Block,
    live_out: &IndexSet<VirtualReg>,
) -> Vec<IndexSet<VirtualReg>> {
    let mut live = live_out.clone();
    for reg in block.terminator.register_uses() {
        live.insert(reg);
    }
    let mut live_at = vec![IndexSet::new(); block.instructions.len() + 1];
    live_at[block.instructions.len()] = live.clone();
    for (index, instruction) in block.instructions.iter().enumerate().rev() {
        for reg in instruction.register_new_defs() {
            live.swap_remove(&reg);
        }
        for reg in instruction.register_uses() {
            live.insert(reg);
        }
        live_at[index] = live.clone();
    }
    live_at
}

fn allocate_spill_slots(
    code: &mut CodeSectionData,
    spill_slots: &mut SpillSlots,
    new_spills: &IndexSet<VirtualReg>,
    graph: &InterferenceGraph,
    functions_blocks: &FunctionBlocks,
) {
    // A register spilled again keeps the slot it has. The value has been in that slot since the
    // round it was first spilled in, which, as below, this round's `graph` knows nothing of: a
    // slot chosen from it could be one whose occupant is live in its slot throughout.
    let new_spills = &new_spills
        .iter()
        .copied()
        .filter(|reg| match reg {
            VirtualReg::R8(reg) => !spill_slots.r8_to_slot.contains_key(*reg),
            VirtualReg::R16(reg) => !spill_slots.r16_to_slot.contains_key(*reg),
            _ => true,
        })
        .collect::<IndexSet<_>>();
    // `graph` only describes the live ranges of this round. An occupant spilled in an earlier
    // round has since been rewritten into short-lived reloads, so the graph no longer has the
    // edges of the range its slot is held for, and it cannot be shown to be free.
    let can_share_slot = |reg: VirtualReg, occupant: VirtualReg| {
        new_spills.contains(&occupant) && !graph.has_edge(reg, occupant)
    };
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
                        .all(|occupant| can_share_slot(VirtualReg::R8(*reg), *occupant))
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
                        .all(|occupant| can_share_slot(VirtualReg::R16(*reg), *occupant));
                    if found {
                        if found_low {
                            true
                        } else {
                            found_low = true;
                            false
                        }
                    } else {
                        found_low = false;
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
                // an r16 spill occupies two consecutive bytes (`slot` and `slot + 1`), so both
                // must be recorded as occupied or a later spill could be placed on top of the
                // high byte, silently clobbering it.
                frame.occupants[slot].push(VirtualReg::R16(*reg));
                frame.occupants[slot + 1].push(VirtualReg::R16(*reg));
                spill_slots.r16_to_slot.insert(
                    *reg,
                    i8::try_from(slot).expect("More than 127 slots required"),
                );
            }
            VirtualReg::Zero(_) | VirtualReg::Carry(_) => {
                unimplemented!("flag spills are handled differently")
            }
            VirtualReg::Call(_) => unimplemented!(
                "VirtualReg::Call is always precolored, so it can never be selected for spilling"
            ),
        }
    }
}

fn rewrite_block_with_spills(
    code: &mut CodeSectionData,
    block_id: BlockId,
    spill_slots: &SpillSlots,
    spills: &IndexSet<VirtualReg>,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
    live_at: &[IndexSet<VirtualReg>],
) {
    let old_instructions = std::mem::take(&mut code.blocks[block_id].instructions);
    let mut new_instructions = Vec::with_capacity(old_instructions.len());
    // Registers that a spill-slot address computation has evicted (via `PUSH`/`POP`) somewhere
    // earlier in this block to avoid colliding with `LD HL, SP+e8`'s hard-wired `HL`/flags
    // clobber - see `begin_spill_slot_address`. Every use of the original register from that
    // point on in the block must be redirected to whatever register now holds its value.
    let mut outer_substitutions = HashMap::<VirtualReg, VirtualReg>::new();

    for (index, instruction) in old_instructions.into_iter().enumerate() {
        let live_here = &live_at[index];
        let defs_requiring_stores = instruction
            .register_defs()
            .filter(|reg| spills.contains(reg))
            .collect::<Vec<_>>();
        let mut reload_substitutions = HashMap::<VirtualReg, VirtualReg>::new();
        match &instruction {
            Instr::Split {
                high, low, source, ..
            } if spills.contains(&VirtualReg::from(*source)) => {
                // if we have a split for which the source requires a spill, we just load directly
                // into the target registers rather than loading and joining only to immediately
                // split. This allows for more flexibility of the targets registers and allows
                // them to be spilled next time instead.
                load_r16_spill_into_high_low(
                    code,
                    block_id,
                    &mut new_instructions,
                    *source,
                    spill_slots,
                    *high,
                    *low,
                    live_here,
                    &mut outer_substitutions,
                );
            }
            _ => {
                for reg in instruction.register_uses() {
                    if spills.contains(&reg) & !reload_substitutions.contains_key(&reg) {
                        let fresh = load_spill_slot(
                            code,
                            block_id,
                            &mut new_instructions,
                            reg,
                            spill_slots,
                            flag_spill_r8s,
                            live_here,
                            &mut outer_substitutions,
                        );
                        reload_substitutions.insert(reg, fresh);
                    }
                }
                // built after the loop above so it reflects any renames those calls just made
                let mut use_substitutions = outer_substitutions.clone();
                use_substitutions.extend(reload_substitutions.clone());

                let defs = instruction.register_new_defs().collect::<Vec<_>>();
                let index = new_instructions.len();
                new_instructions.push(rewrite_instruction(instruction, &use_substitutions));
                for reg in defs {
                    // additional instructions are added due to spills, so the index changes
                    // and as a result the defined needs to be patched
                    patch_reg_defined(code, reg, block_id, index);
                }
            }
        }

        for reg in defs_requiring_stores {
            // an instruction which modifies a spilled register in place was rewritten to modify
            // the register it was reloaded into, so that is the one holding the new value
            let value = reload_substitutions.get(&reg).copied().unwrap_or(reg);
            store_spill_slot(
                code,
                block_id,
                &mut new_instructions,
                reg,
                value,
                spill_slots,
                flag_spill_r8s,
                &live_at[index + 1],
                &mut outer_substitutions,
            );
        }
    }

    let mut reload_substitutions = HashMap::<VirtualReg, VirtualReg>::new();

    let old_terminator = std::mem::replace(&mut code.blocks[block_id].terminator, Terminator::Ret);
    let live_here = &live_at[live_at.len() - 1];
    for reg in old_terminator.register_uses() {
        if spills.contains(&reg) & !reload_substitutions.contains_key(&reg) {
            let fresh = load_spill_slot(
                code,
                block_id,
                &mut new_instructions,
                reg,
                spill_slots,
                flag_spill_r8s,
                live_here,
                &mut outer_substitutions,
            );
            reload_substitutions.insert(reg, fresh);
        }
    }
    let mut use_substitutions = outer_substitutions;
    use_substitutions.extend(reload_substitutions);

    code.blocks[block_id].instructions = new_instructions;
    code.blocks[block_id].terminator = rewrite_terminator(old_terminator, &use_substitutions);
}

fn patch_reg_defined(code: &mut CodeSectionData, reg: VirtualReg, block_id: BlockId, index: usize) {
    code.register_data_mut(reg).defined.insert(block_id, index);
}

/// Already-precolored registers whose live range was found spanning a spill-slot address
/// computation and were evicted (via `PUSH`) to make room for it; see
/// [`begin_spill_slot_address`].
struct SpillAddressGuard {
    hl: Option<R16Id>,
    a: Option<AId>,
    z: Option<ZeroId>,
    c: Option<CarryId>,
}

fn resolve_current(
    outer_substitutions: &HashMap<VirtualReg, VirtualReg>,
    reg: VirtualReg,
) -> VirtualReg {
    outer_substitutions.get(&reg).copied().unwrap_or(reg)
}

/// Emits the `LD HL, SP+e8` used to compute a spill slot's address. `LD HL, SP+e8` is the only
/// Game Boy instruction that can do SP-relative addressing, and it hard-wires its target to `HL`
/// and always clobbers the flags - so if some other already-precolored `HL`/`A`/`Fz`/`Fc`
/// register (found live in `live_here`) spans this exact point, it is first evicted with a
/// `PUSH` and restored afterward via [`end_spill_slot_address`], rather than being left to
/// silently collide with the fresh precolored registers this instruction needs.
fn begin_spill_slot_address(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    slot: i8,
    live_here: &IndexSet<VirtualReg>,
    outer_substitutions: &HashMap<VirtualReg, VirtualReg>,
) -> (HLId, SpillAddressGuard) {
    let hl = live_here.iter().copied().find_map(|reg| match reg {
        VirtualReg::R16(r16) if code.r16_registers[r16].color == Some(Color::HL) => Some(r16),
        _ => None,
    });
    let a = live_here.iter().copied().find_map(|reg| match reg {
        VirtualReg::R8(r8) if code.r8_registers[r8].color == Some(Color::A) => {
            Some(r8.downcast_to_a_without_move())
        }
        _ => None,
    });
    let z = live_here.iter().copied().find_map(|reg| match reg {
        VirtualReg::Zero(z) if code.zero_registers[z].color == Some(Color::Fz) => Some(z),
        _ => None,
    });
    let c = live_here.iter().copied().find_map(|reg| match reg {
        VirtualReg::Carry(c) if code.carry_registers[c].color == Some(Color::Fc) => Some(c),
        _ => None,
    });

    let mut growth: i8 = 0;
    if let Some(hl) = hl {
        let VirtualReg::R16(current) = resolve_current(outer_substitutions, VirtualReg::R16(hl))
        else {
            unreachable!("R16 substitutes to non-R16")
        };
        instructions.push(Instr::PushR16 { source: current });
        growth += 2;
    }
    if a.is_some() || z.is_some() || c.is_some() {
        let source = a.map(|a| {
            let VirtualReg::R8(current) = resolve_current(
                outer_substitutions,
                VirtualReg::R8(a.upcast_to_r8_without_move()),
            ) else {
                unreachable!("R8 substitutes to non-R8")
            };
            current.downcast_to_a_without_move()
        });
        let z_in = z.map(|z| {
            let VirtualReg::Zero(current) =
                resolve_current(outer_substitutions, VirtualReg::Zero(z))
            else {
                unreachable!("Zero substitutes to non-Zero")
            };
            current
        });
        let c_in = c.map(|c| {
            let VirtualReg::Carry(current) =
                resolve_current(outer_substitutions, VirtualReg::Carry(c))
            else {
                unreachable!("Carry substitutes to non-Carry")
            };
            current
        });
        instructions.push(Instr::PushAF { source, z_in, c_in });
        growth += 2;
    }

    let target = code
        .r16_registers
        .insert(Register {
            color: Some(Color::HL),
            defined: [(block_id, instructions.len())].into_iter().collect(),
        })
        .downcast_to_hl_without_move();
    let z_out = code.zero_registers.insert(Register {
        color: Some(Color::Fz),
        defined: [(block_id, instructions.len())].into_iter().collect(),
    });
    let c_out = code.carry_registers.insert(Register {
        color: Some(Color::Fc),
        defined: [(block_id, instructions.len())].into_iter().collect(),
    });
    instructions.push(Instr::LdSPE8 {
        target,
        z: z_out,
        c: c_out,
        right: slot
            .checked_add(growth)
            .expect("spill slot offset overflowed i8 after accounting for PUSH HL/AF growth"),
    });
    (target, SpillAddressGuard { hl, a, z, c })
}

/// Restores whatever [`begin_spill_slot_address`] evicted, and records the fresh registers now
/// holding those values in `outer_substitutions` so later uses in this block get redirected to
/// them by [`rewrite_instruction`]/[`rewrite_terminator`].
fn end_spill_slot_address(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    guard: SpillAddressGuard,
    outer_substitutions: &mut HashMap<VirtualReg, VirtualReg>,
) {
    if guard.a.is_some() || guard.z.is_some() || guard.c.is_some() {
        let a = code
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
        instructions.push(Instr::PopAF { target: a, z, c });
        if let Some(outer_a) = guard.a {
            outer_substitutions.insert(
                VirtualReg::R8(outer_a.upcast_to_r8_without_move()),
                VirtualReg::R8(a.upcast_to_r8_without_move()),
            );
        }
        if let Some(outer_z) = guard.z {
            outer_substitutions.insert(VirtualReg::Zero(outer_z), VirtualReg::Zero(z));
        }
        if let Some(outer_c) = guard.c {
            outer_substitutions.insert(VirtualReg::Carry(outer_c), VirtualReg::Carry(c));
        }
    }
    if let Some(outer_hl) = guard.hl {
        let target = code.r16_registers.insert(Register {
            color: Some(Color::HL),
            defined: [(block_id, instructions.len())].into_iter().collect(),
        });
        instructions.push(Instr::PopR16 { target });
        outer_substitutions.insert(VirtualReg::R16(outer_hl), VirtualReg::R16(target));
    }
}

#[allow(clippy::too_many_arguments)]
fn load_spill_slot(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    reg: VirtualReg,
    spill_slots: &SpillSlots,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
    live_here: &IndexSet<VirtualReg>,
    outer_substitutions: &mut HashMap<VirtualReg, VirtualReg>,
) -> VirtualReg {
    match reg {
        VirtualReg::R8(r8) => {
            let slot = spill_slots.r8_to_slot[r8];
            let (source_addr, guard) = begin_spill_slot_address(
                code,
                block_id,
                instructions,
                slot,
                live_here,
                outer_substitutions,
            );
            let target = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Ld8Ptr {
                target,
                source_addr: Addr::Reg(source_addr.upcast_to_r16_without_move()),
            });
            end_spill_slot_address(code, block_id, instructions, guard, outer_substitutions);
            VirtualReg::R8(target)
        }
        VirtualReg::R16(r16) => {
            let low_in = code.r8_registers.insert(Register::default());
            let high_in = code.r8_registers.insert(Register::default());
            load_r16_spill_into_high_low(
                code,
                block_id,
                instructions,
                r16,
                spill_slots,
                high_in,
                low_in,
                live_here,
                outer_substitutions,
            );
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
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            instructions.push(Instr::Ld8Value {
                target: a.upcast_to_r8_without_move(),
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
            // zero flag is stored as 0xff when set and 0x00 when not set. To set it back to the
            // zero flag we need to take the complement of the value, but CPL doesn't set the zero
            // flag so XOR is the next best instruction.
            instructions.push(Instr::Xor {
                z,
                c,
                left: a,
                right: Operand8::Reg(source),
            });
            VirtualReg::Zero(z)
        }
        VirtualReg::Carry(flag) => {
            let source = flag_spill_r8s[&VirtualReg::Carry(flag)];
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            // this is safe to do without a move as the register will contain 0xFF if true
            // and 0x00 if false so rotating it with RRC doesn't change anything, it just
            // sets the z or c flags
            instructions.push(Instr::Rrc { z, c, source });
            VirtualReg::Carry(c)
        }
        VirtualReg::Call(_) => unimplemented!(
            "VirtualReg::Call is always precolored, so it can never be selected for spilling"
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn load_r16_spill_into_high_low(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    r16: R16Id,
    spill_slots: &SpillSlots,
    high: R8Id,
    low: R8Id,
    live_here: &IndexSet<VirtualReg>,
    outer_substitutions: &mut HashMap<VirtualReg, VirtualReg>,
) {
    let slot = spill_slots.r16_to_slot[r16];
    let (source_addr, guard) = begin_spill_slot_address(
        code,
        block_id,
        instructions,
        slot,
        live_here,
        outer_substitutions,
    );
    let a = code
        .r8_registers
        .insert(Register {
            color: Some(Color::A),
            defined: [(block_id, instructions.len())].into_iter().collect(),
        })
        .downcast_to_a_without_move();
    instructions.push(Instr::Ld8Inc {
        target: a,
        source_addr,
    });
    code.r8_registers[low]
        .defined
        .insert(block_id, instructions.len());
    instructions.push(Instr::Ld8 {
        target: low,
        source: a.upcast_to_r8_without_move(),
    });
    code.r8_registers[high]
        .defined
        .insert(block_id, instructions.len());
    instructions.push(Instr::Ld8Ptr {
        target: high,
        source_addr: Addr::Reg(source_addr.upcast_to_r16_without_move()),
    });
    end_spill_slot_address(code, block_id, instructions, guard, outer_substitutions);
}

#[allow(clippy::too_many_arguments)]
fn store_spill_slot(
    code: &mut CodeSectionData,
    block_id: BlockId,
    instructions: &mut Vec<Instr>,
    reg: VirtualReg,
    value: VirtualReg,
    spill_slots: &SpillSlots,
    flag_spill_r8s: &IndexMap<VirtualReg, R8Id>,
    live_here: &IndexSet<VirtualReg>,
    outer_substitutions: &mut HashMap<VirtualReg, VirtualReg>,
) {
    match reg {
        VirtualReg::R8(r8) => {
            let VirtualReg::R8(source) = value else {
                panic!("{r8:?} is stored from non-r8 {value:?}")
            };
            let slot = spill_slots.r8_to_slot[r8];
            let (target_addr, guard) = begin_spill_slot_address(
                code,
                block_id,
                instructions,
                slot,
                live_here,
                outer_substitutions,
            );
            instructions.push(Instr::Store {
                addr: Addr::Reg(target_addr.upcast_to_r16_without_move()),
                source,
            });
            end_spill_slot_address(code, block_id, instructions, guard, outer_substitutions);
        }
        VirtualReg::R16(r16) => {
            let VirtualReg::R16(source) = value else {
                panic!("{r16:?} is stored from non-r16 {value:?}")
            };
            let slot = spill_slots.r16_to_slot[r16];
            let (target_addr, guard) = begin_spill_slot_address(
                code,
                block_id,
                instructions,
                slot,
                live_here,
                outer_substitutions,
            );
            let low = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let high = code.r8_registers.insert(Register {
                color: None,
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Split { high, low, source });
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
            instructions.push(Instr::StoreInc {
                addr: target_addr,
                source: a,
            });
            instructions.push(Instr::Store {
                addr: Addr::Reg(target_addr.upcast_to_r16_without_move()),
                source: high,
            });
            end_spill_slot_address(code, block_id, instructions, guard, outer_substitutions);
        }
        VirtualReg::Zero(z_in) => {
            // zero flags are difficult, unfortunately. We need to push them onto
            // the stack then pop onto a 16-bit register
            instructions.push(Instr::PushAF {
                source: None,
                z_in: Some(z_in),
                c_in: None,
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
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c_in = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Rlc {
                z,
                c: c_in,
                source: low,
            });
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            instructions.push(Instr::FreshA { target: a });
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Sbc {
                z,
                c,
                left: a,
                right: Operand8::Reg(a.upcast_to_r8_without_move()),
                c_in,
            });
            let target = flag_spill_r8s[&VirtualReg::Zero(z_in)];
            code.r8_registers[target]
                .defined
                .insert(block_id, instructions.len());
            instructions.push(Instr::Ld8 {
                target,
                source: a.upcast_to_r8_without_move(),
            });
        }
        VirtualReg::Carry(c_in) => {
            let a = code
                .r8_registers
                .insert(Register {
                    color: Some(Color::A),
                    defined: [(block_id, instructions.len())].into_iter().collect(),
                })
                .downcast_to_a_without_move();
            instructions.push(Instr::FreshA { target: a });
            let z = code.zero_registers.insert(Register {
                color: Some(Color::Fz),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            let c = code.carry_registers.insert(Register {
                color: Some(Color::Fc),
                defined: [(block_id, instructions.len())].into_iter().collect(),
            });
            instructions.push(Instr::Sbc {
                z,
                c,
                left: a,
                right: Operand8::Reg(a.upcast_to_r8_without_move()),
                c_in,
            });
            let target = flag_spill_r8s[&VirtualReg::Carry(c_in)];
            code.r8_registers[target]
                .defined
                .insert(block_id, instructions.len());
            instructions.push(Instr::Ld8 {
                target,
                source: a.upcast_to_r8_without_move(),
            });
        }
        VirtualReg::Call(_) => unimplemented!(
            "VirtualReg::Call is always precolored, so it can never be selected for spilling"
        ),
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
        | Instr::Breakpoint
        | Instr::Ld8Value { .. }
        | Instr::Ld16 { .. }
        | Instr::LdSPValue { .. }
        | Instr::Scf { .. }
        | Instr::AddSP { .. }
        | Instr::LdSPE8 { .. }
        | Instr::PopR16 { .. }
        | Instr::PopAF { .. }
        | Instr::LdReturn8 { .. }
        | Instr::LdReturn16 { .. }
        | Instr::FreshA { .. } => {}
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
            addr: target_addr,
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
        Instr::Rla { source, c_in, .. } | Instr::Rra { source, c_in, .. } => {
            *source = sub_a(*source);
            *c_in = sub_carry(*c_in);
        }
        Instr::Rlca { source, .. } | Instr::Rrca { source, .. } => {
            *source = sub_a(*source);
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
        | Instr::DecPair { source, .. }
        | Instr::IncPair { source, .. }
        | Instr::PushR16 { source }
        | Instr::Split { source, .. } => {
            *source = sub_r16(*source);
        }
        Instr::PushAF { source, z_in, c_in } => {
            if let Some(source) = source {
                *source = sub_a(*source);
            }
            if let Some(z_in) = z_in {
                *z_in = sub_zero(*z_in);
            }
            if let Some(c_in) = c_in {
                *c_in = sub_carry(*c_in);
            }
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
        Instr::BulkCopy {
            target_addr,
            source_addr,
            count,
            ..
        } => {
            *target_addr = sub_r16(*target_addr);
            *source_addr = sub_hl(*source_addr);
            *count = sub_r16(*count);
        }
        Instr::BulkFill {
            addr, count, value, ..
        } => {
            *addr = sub_hl(*addr);
            *count = sub_r16(*count);
            *value = sub_a(*value);
        }
    }
    instruction
}

fn rewrite_terminator(
    mut terminator: Terminator,
    use_substitutions: &HashMap<VirtualReg, VirtualReg>,
) -> Terminator {
    if use_substitutions.is_empty() {
        return terminator;
    }
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
    match &mut terminator {
        Terminator::Call { .. }
        | Terminator::Jp { .. }
        | Terminator::JpFn { .. }
        | Terminator::Ret
        | Terminator::Reti
        | Terminator::Halt { .. } => {}
        Terminator::CallR16 { callee, .. } => {
            *callee = sub_r16(*callee);
        }
        Terminator::CallIf { source, .. }
        | Terminator::JpIf { source, .. }
        | Terminator::RetIf { source, .. } => match source {
            FlagId::Zero(source) => {
                *source = sub_zero(*source);
            }
            FlagId::Carry(source) => {
                *source = sub_carry(*source);
            }
        },
    }
    terminator
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
        for index in 1..code.blocks[entry].instructions.len() {
            for reg in code.blocks[entry].instructions[index]
                .register_new_defs()
                .collect::<Vec<_>>()
            {
                code.register_data_mut(reg).defined[entry] = index;
            }
        }
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

#[derive(Default)]
struct Frame {
    occupants: Vec<Vec<VirtualReg>>,
}

#[derive(Default)]
struct SpillSlots {
    function_to_frame: SparseSecondaryMap<BlockId, Frame>,
    r8_to_slot: SparseSecondaryMap<R8Id, i8>,
    r16_to_slot: SparseSecondaryMap<R16Id, i8>,
    /// Registers which spilling can do nothing more for: those already spilled, and those
    /// created to carry a value between a spill slot and the instruction it is needed by.
    short_lived: IndexSet<VirtualReg>,
}

#[derive(Default)]
struct FunctionBlocks {
    block_to_function: SparseSecondaryMap<BlockId, BlockId>,
    function_to_exits: SparseSecondaryMap<BlockId, StableSparseSlotKeySet<BlockId>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Identifier;
    use crate::codegen::interference::build_interference_graph;
    use crate::lir::Ld16Source;
    use slotmap::SlotMap;

    /// Builds a single-block, single-function [`CodeSectionData`] containing two `r16`
    /// registers (`x`, `y`) which are defined and then both used together by a later
    /// instruction (`PushR16` each, with `x`'s use ordered after `y`'s definition), so that
    /// a real interference edge is established between them via [`build_interference_graph`]
    /// exactly as it would be for genuinely simultaneously-live locals.
    fn two_interfering_r16_regs() -> (CodeSectionData, R16Id, R16Id) {
        let mut code = CodeSectionData {
            region: None,
            addr: None,
            functions: SparseSecondaryMap::new(),
            blocks: SlotMap::with_key(),
            zero_registers: SlotMap::with_key(),
            carry_registers: SlotMap::with_key(),
            r8_registers: SlotMap::with_key(),
            r16_registers: SlotMap::with_key(),
        };
        let x = code.r16_registers.insert(Register::default());
        let y = code.r16_registers.insert(Register::default());
        let block = code.blocks.insert(Block {
            instructions: Vec::new(),
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        code.functions.insert(block, Identifier::from("f"));
        code.blocks[block].instructions = vec![
            Instr::Ld16 {
                target: x,
                source: Ld16Source::Literal(0),
            },
            Instr::Ld16 {
                target: y,
                source: Ld16Source::Literal(0),
            },
            Instr::PushR16 { source: x },
            Instr::PushR16 { source: y },
        ];
        code.r16_registers[x].defined.insert(block, 0);
        code.r16_registers[y].defined.insert(block, 1);
        (code, x, y)
    }

    #[test]
    fn allocate_spill_slots_gives_interfering_r16_regs_disjoint_byte_ranges() {
        let (code, x, y) = two_interfering_r16_regs();
        let liveness = calculate_block_liveness(&code);
        let (graph, _moves) = build_interference_graph(&code, &liveness);
        assert!(
            graph.has_edge(VirtualReg::R16(x), VirtualReg::R16(y)),
            "test setup should make x and y interfere"
        );

        let functions_blocks = calculate_function_blocks(&code);
        let mut spill_slots = SpillSlots::default();
        let new_spills = [VirtualReg::R16(x), VirtualReg::R16(y)]
            .into_iter()
            .collect::<IndexSet<_>>();
        let mut code = code;
        allocate_spill_slots(
            &mut code,
            &mut spill_slots,
            &new_spills,
            &graph,
            &functions_blocks,
        );

        let x_slot = spill_slots.r16_to_slot[x];
        let y_slot = spill_slots.r16_to_slot[y];
        // each r16 spill occupies two consecutive bytes (`slot`, `slot + 1`); interfering
        // registers must never share a byte, or one's spill store/load will silently
        // clobber the other's.
        let x_range = x_slot..=(x_slot + 1);
        let y_range = y_slot..=(y_slot + 1);
        assert!(
            !x_range.contains(&y_slot) && !x_range.contains(&(y_slot + 1)),
            "x's slot range {x_range:?} overlaps y's byte range {y_range:?} for interfering r16 regs"
        );
    }

    #[test]
    fn allocate_spill_slots_gives_r16_two_adjacent_free_bytes() {
        // `a`, `b` and `c` are live together so take a byte each, in that order. `z` is live
        // alongside `b` alone, so it may share the bytes of `a` and `c`, but they are not next to
        // each other: it has to start a pair of its own after `c` rather than straddle `b`.
        let mut code = CodeSectionData {
            region: None,
            addr: None,
            functions: SparseSecondaryMap::new(),
            blocks: SlotMap::with_key(),
            zero_registers: SlotMap::with_key(),
            carry_registers: SlotMap::with_key(),
            r8_registers: SlotMap::with_key(),
            r16_registers: SlotMap::with_key(),
        };
        let a = code.r8_registers.insert(Register::default());
        let b = code.r8_registers.insert(Register::default());
        let c = code.r8_registers.insert(Register::default());
        let z = code.r16_registers.insert(Register::default());
        let block = code.blocks.insert(Block {
            instructions: Vec::new(),
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        code.functions.insert(block, Identifier::from("f"));
        code.blocks[block].instructions = vec![
            Instr::Ld8Value {
                target: a,
                source: 0,
            },
            Instr::Ld8Value {
                target: c,
                source: 0,
            },
            Instr::Ld8Value {
                target: b,
                source: 0,
            },
            Instr::Store {
                addr: Addr::Literal(0xC000),
                source: a,
            },
            Instr::Store {
                addr: Addr::Literal(0xC000),
                source: c,
            },
            Instr::Ld16 {
                target: z,
                source: Ld16Source::Literal(0),
            },
            Instr::Store {
                addr: Addr::Literal(0xC000),
                source: b,
            },
            Instr::PushR16 { source: z },
        ];
        code.r8_registers[a].defined.insert(block, 0);
        code.r8_registers[c].defined.insert(block, 1);
        code.r8_registers[b].defined.insert(block, 2);
        code.r16_registers[z].defined.insert(block, 5);

        let liveness = calculate_block_liveness(&code);
        let (graph, _moves) = build_interference_graph(&code, &liveness);
        let (a, b, c, z) = (
            VirtualReg::R8(a),
            VirtualReg::R8(b),
            VirtualReg::R8(c),
            VirtualReg::R16(z),
        );
        assert!(
            graph.has_edge(a, b) && graph.has_edge(b, c) && graph.has_edge(a, c),
            "test setup should make a, b and c interfere"
        );
        assert!(
            graph.has_edge(z, b) && !graph.has_edge(z, a) && !graph.has_edge(z, c),
            "test setup should make z interfere with b alone"
        );

        let functions_blocks = calculate_function_blocks(&code);
        let mut spill_slots = SpillSlots::default();
        let new_spills = [a, b, c, z].into_iter().collect::<IndexSet<_>>();
        allocate_spill_slots(
            &mut code,
            &mut spill_slots,
            &new_spills,
            &graph,
            &functions_blocks,
        );

        let VirtualReg::R8(b) = b else { unreachable!() };
        let VirtualReg::R16(z) = z else {
            unreachable!()
        };
        let b_slot = spill_slots.r8_to_slot[b];
        let z_slot = spill_slots.r16_to_slot[z];
        assert!(
            z_slot != b_slot && z_slot + 1 != b_slot,
            "z's bytes {z_slot} and {} overlap b's byte {b_slot}",
            z_slot + 1
        );
    }

    #[test]
    fn rewrite_terminator_substitutes_call_r16_callee() {
        // regression test: `rewrite_terminator` previously had no `CallR16` arm at all, so a
        // spilled callee register would be left referring to the old (now-dead) virtual
        // register instead of the freshly reloaded one.
        let mut builder = crate::lir::CodeSectionBuilder::default();
        let entry = builder.new_function(Identifier::from("f"));
        let old = builder.insert_ld16(entry, crate::lir::Ld16Source::Literal(0));
        let new = builder.insert_ld16(entry, crate::lir::Ld16Source::Literal(0));

        let mut use_substitutions = HashMap::new();
        use_substitutions.insert(VirtualReg::R16(old), VirtualReg::R16(new));

        let terminator = Terminator::CallR16 {
            callee: old,
            return_block: entry,
        };
        let rewritten = rewrite_terminator(terminator, &use_substitutions);
        assert!(
            matches!(rewritten, Terminator::CallR16 { callee, .. } if callee == new),
            "{rewritten:?} should have substituted callee {old:?} for {new:?}"
        );
    }
}
