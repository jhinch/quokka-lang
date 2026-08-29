use crate::interference::{
    BlockLiveness, InterferenceGraph, build_interference_graph, calculate_block_liveness,
};
use crate::lir::{BlockId, CodeSectionData, VirtualReg};
use crate::reg::{Color, ColorSet};
use indexmap::IndexSet;
use std::collections::{HashMap, VecDeque};
use tracing::instrument;

#[instrument(skip_all)]
pub fn allocate_registers(code: &mut CodeSectionData) -> HashMap<VirtualReg, Color> {
    let max_iterations =
        code.r8_registers.len() + code.r16_registers.len() + code.flag_registers.len() + 100;
    for i in 0..max_iterations {
        if let Some(colors) = allocate_registers_round(code, i) {
            return colors;
        }
    }
    panic!("Failed to allocate registers")
}

#[instrument(skip_all, fields(i = %i))]
pub fn allocate_registers_round(
    code: &mut CodeSectionData,
    i: usize,
) -> Option<HashMap<VirtualReg, Color>> {
    let block_liveness = calculate_block_liveness(code);
    match try_allocate_registers(code, &block_liveness) {
        Ok(colors) => Some(colors),
        Err(spills) => {
            rewrite_code(code, spills, block_liveness);
            None
        }
    }
}

#[instrument(skip_all)]
fn try_allocate_registers(
    code: &CodeSectionData,
    block_liveness: &BlockLiveness,
) -> Result<HashMap<VirtualReg, Color>, IndexSet<VirtualReg>> {
    let (graph, moves) = build_interference_graph(code, block_liveness);
    let allocator = RegAllocator {
        graph,
        worklist_moves: moves,
        ..Default::default()
    };
    allocator.run(code)
}

#[instrument(skip_all)]
fn rewrite_code(
    _code: &mut CodeSectionData,
    _spills: IndexSet<VirtualReg>,
    _block_liveness: BlockLiveness,
) {
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
    ) -> Result<HashMap<VirtualReg, Color>, IndexSet<VirtualReg>> {
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
    ) -> Result<HashMap<VirtualReg, Color>, IndexSet<VirtualReg>> {
        let mut colors = HashMap::<VirtualReg, Color>::new();
        for reg in code
            .r8_registers
            .keys()
            .map(VirtualReg::R8)
            .chain(code.r16_registers.keys().map(VirtualReg::R16))
        {
            if let Some(color) = self.graph.precolor(reg) {
                colors.insert(reg, color);
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
            Err(self.spilled_nodes)
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
}
