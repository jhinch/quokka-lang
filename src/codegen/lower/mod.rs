use super::LoweredGlobal;
use crate::lir::{BlockId, CarryId, CodeSectionBuilder, FnAddr, R8Id, ZeroId};
use crate::mir;
use crate::num::u3;
use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use indexmap::map::Entry;
use slotmap::{SecondaryMap, SparseSecondaryMap};

mod bulk;
mod compare;
mod place;
mod scalar16;
mod scalar8;
mod terminator;
mod u1;

/// Everything needed to lower one MIR function to LIR. Built once per function.
pub(super) struct InstrCtx<'a> {
    ty_registry: &'a TyRegistry,
    mir: &'a mir::Mir,
    function: &'a mir::Function,
    globals: &'a SecondaryMap<mir::GlobalId, LoweredGlobal>,
    functions: &'a SecondaryMap<mir::FunctionId, FnAddr>,
    blocks: SecondaryMap<mir::BlockId, BlockId>,
    lir: &'a mut CodeSectionBuilder,
    locals: Locals,
}

impl<'a> InstrCtx<'a> {
    pub(super) fn new(
        ty_registry: &'a TyRegistry,
        mir: &'a mir::Mir,
        function: &'a mir::Function,
        start_block: BlockId,
        globals: &'a SecondaryMap<mir::GlobalId, LoweredGlobal>,
        functions: &'a SecondaryMap<mir::FunctionId, FnAddr>,
        lir: &'a mut CodeSectionBuilder,
    ) -> Self {
        let mut blocks = SecondaryMap::new();
        blocks.insert(function.start_block, start_block);
        for mir_block in function.traverse_blocks() {
            blocks.insert(
                mir_block,
                if function.start_block == mir_block {
                    start_block
                } else {
                    lir.new_block()
                },
            );
        }
        InstrCtx {
            ty_registry,
            mir,
            function,
            globals,
            functions,
            blocks,
            lir,
            locals: Locals::default(),
        }
    }

    pub(super) fn generate_from_function(mut self) {
        let function = self.function;
        for mir_block in function.traverse_blocks() {
            for instruction in &function.blocks[mir_block].instructions {
                if let mir::Instr::Phi { target, sources } = instruction {
                    match function.locals[*target]
                        .ty
                        .bit_size(self.ty_registry)
                        .into_u32()
                    {
                        1 => {
                            let r8 = self.lir.reserve_phi();
                            self.locals.phi_r1s.insert(*target, (r8, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = self.blocks[mir_block];
                                match self
                                    .locals
                                    .phi_source_r1s
                                    .entry(block)
                                    .unwrap()
                                    .or_default()
                                    .entry(*local)
                                {
                                    Entry::Occupied(_) => panic!(
                                        "block {mir_block:?} already has phi r8 registered for {local:?}"
                                    ),
                                    Entry::Vacant(entry) => {
                                        entry.insert(r8);
                                    }
                                }
                                self.locals.phi_r1s[target].1.push(block);
                            }
                        }
                        8 => {
                            let r8 = self.lir.reserve_phi();
                            self.locals.phi_r8s.insert(*target, (r8, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = self.blocks[mir_block];
                                match self
                                    .locals
                                    .phi_source_r8s
                                    .entry(block)
                                    .unwrap()
                                    .or_default()
                                    .entry(*local)
                                {
                                    Entry::Occupied(_) => panic!(
                                        "block {mir_block:?} already has phi r8 registered for {local:?}"
                                    ),
                                    Entry::Vacant(entry) => {
                                        entry.insert(r8);
                                    }
                                }
                                self.locals.phi_r8s[target].1.push(block);
                            }
                        }
                        16 => {
                            let low = self.lir.reserve_phi();
                            let high = self.lir.reserve_phi();
                            self.locals
                                .phi_r16s
                                .insert(*target, (high, low, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = self.blocks[mir_block];
                                match self
                                    .locals
                                    .phi_source_r16s
                                    .entry(block)
                                    .unwrap()
                                    .or_default()
                                    .entry(*local)
                                {
                                    Entry::Occupied(_) => panic!(
                                        "block {mir_block:?} already has phi r8 pair registered for {local:?}"
                                    ),
                                    Entry::Vacant(entry) => {
                                        entry.insert((high, low));
                                    }
                                }
                                self.locals.phi_r16s[target].2.push(block);
                            }
                        }
                        _ => unimplemented!(
                            "{} cannot fit in a 8-bit or 16-bit register",
                            function.locals[*target].ty.display(self.ty_registry)
                        ),
                    }
                }
            }
        }
        for mir_block in function.traverse_blocks() {
            let block = self.blocks[mir_block];
            for instruction in &function.blocks[mir_block].instructions {
                match instruction {
                    mir::Instr::Nop => {
                        // currently we don't translate Nop in MIR to Nop in LIR
                    }
                    mir::Instr::Breakpoint => self.lir.insert_breakpoint(block),
                    mir::Instr::Transfer { .. } | mir::Instr::Fill { .. } => {
                        self.generate_from_transfer_or_fill(instruction, block);
                    }
                    _ => {
                        let ty = self.ty_from_instr(instruction);
                        self.generate_from_instr(instruction, ty, block);
                    }
                }
            }
            // `r1_to_r8` needs all of `self`, so the sources are copied out of `locals` first
            let phi_source_r1s: Vec<(mir::LocalId, R8Id)> = self
                .locals
                .phi_source_r1s
                .get(block)
                .into_iter()
                .flatten()
                .map(|(&local, &r8)| (local, r8))
                .collect();
            for (local, r8) in phi_source_r1s {
                let (r1, condition) = self.locals.r1s[local];
                let mut source = self.r1_to_r8(r1, block);
                if condition != mir::Condition::NotZero {
                    let a = self.lir.insert_ld_a(block, source);
                    self.lir.insert_cpl(block, a);
                    source = self.lir.insert_move_a(block, a);
                }
                self.lir.insert_phi_ld(block, r8, source);
            }
            if let Some(phis) = self.locals.phi_source_r8s.get(block) {
                for (local, &r8) in phis {
                    self.lir.insert_phi_ld(block, r8, self.locals.r8s[*local]);
                }
            }
            if let Some(phis) = self.locals.phi_source_r16s.get(block) {
                for (local, &(target_high, target_low)) in phis {
                    let (source_high, source_low) = self.locals.r16s[*local];
                    self.lir.insert_phi_ld(block, target_low, source_low);
                    self.lir.insert_phi_ld(block, target_high, source_high);
                }
            }
            self.generate_from_terminator(&function.blocks[mir_block].terminator, block);
        }
        for (r8, blocks) in self.locals.phi_r1s.values() {
            self.lir.complete_phi(*r8, blocks.as_slice());
        }
        for (r8, blocks) in self.locals.phi_r8s.values() {
            self.lir.complete_phi(*r8, blocks.as_slice());
        }
        for (high, low, blocks) in self.locals.phi_r16s.values() {
            self.lir.complete_phi(*low, blocks.as_slice());
            self.lir.complete_phi(*high, blocks.as_slice());
        }
    }

    fn ty_from_instr(&self, instruction: &mir::Instr) -> Ty {
        match (instruction.target_local(), instruction) {
            (Some(target), _) => self.function.locals[target].ty,
            (
                None,
                mir::Instr::Store {
                    target,
                    projections,
                    ..
                },
            ) => self.projected_ty(*target, projections),
            (None, _) => unreachable!("{instruction:?}"),
        }
    }

    /// generate LIR from the MIR instruction.
    ///
    /// An additional move after every lowered MIR instruction to ensure that between
    /// A MIR instruction will never have a pre-colored source as this would mean that there
    /// is likely interference due to the restrictive machine instructions.
    /// Unnecessary moves will be coalesced during register allocation if not needed.
    fn generate_from_instr(&mut self, instruction: &mir::Instr, ty: Ty, block: BlockId) {
        match ty {
            Ty::U1 => {
                if let Some((target, mut r1, mut condition)) =
                    self.generate_from_u1_instr(instruction, block)
                {
                    if self.function.locals[target].is_return {
                        let r8 = self.r1_to_r8(r1, block);
                        let a = self.lir.insert_ld_a(block, r8);
                        if condition != mir::Condition::NotZero {
                            self.lir.insert_cpl(block, a);
                            condition = mir::Condition::NotZero;
                        }
                        r1 = R1::R8(a.upcast_to_r8_without_move());
                    }
                    self.locals.r1s.insert(target, (r1, condition));
                }
            }
            Ty::U2 => todo!("{instruction:?} {:?}", Ty::U2),
            Ty::U4 => todo!("{instruction:?} {:?}", Ty::U4),
            Ty::U8 | Ty::I8 => {
                if let Some((target, mut r8)) = self.generate_from_scalar8_instr(instruction, block)
                {
                    if self.function.locals[target].is_return {
                        let a = self.lir.insert_ld_a(block, r8);
                        r8 = a.upcast_to_r8_without_move();
                    }
                    self.locals.r8s.insert(target, r8);
                }
            }
            Ty::U16 | Ty::I16 => {
                if let Some((target, mut high, mut low)) =
                    self.generate_from_scalar16_instr(instruction, block)
                {
                    if self.function.locals[target].is_return {
                        let (b, c) = self.lir.insert_ld_bc(block, high, low);
                        high = b;
                        low = c;
                    }
                    self.locals.r16s.insert(target, (high, low));
                }
            }
            ty if ty.is_struct() || ty.is_array() || ty.is_fn_ptr() => match instruction {
                mir::Instr::Alloc { .. }
                | mir::Instr::ReturnValue { .. }
                | mir::Instr::Phi { .. }
                | mir::Instr::Load { .. }
                | mir::Instr::Store { .. }
                | mir::Instr::Copy { .. }
                | mir::Instr::BitCast { .. }
                | mir::Instr::Read { .. }
                | mir::Instr::Write { .. }
                | mir::Instr::Eq { .. }
                | mir::Instr::Ne { .. } => match ty.bit_size(self.ty_registry).into_u32() {
                    8 => {
                        if let Some((target, r8)) =
                            self.generate_from_scalar8_instr(instruction, block)
                        {
                            self.locals.r8s.insert(target, r8);
                        }
                    }
                    16 => {
                        if let Some((target, high, low)) =
                            self.generate_from_scalar16_instr(instruction, block)
                        {
                            self.locals.r16s.insert(target, (high, low));
                        }
                    }
                    _ => unimplemented!(
                        "Valid MIR should never produce structs, arrays or fn ptrs which are not 8 or 16-bit"
                    ),
                },
                _ => unimplemented!(
                    "Only alloc, equality, reads and writes are supported on structs, arrays and fn ptrs"
                ),
            },
            ty => unimplemented!("{ty:?}"),
        }
    }
}

#[derive(Default)]
struct Locals {
    r1s: SparseSecondaryMap<mir::LocalId, (R1, mir::Condition)>,
    r8s: SparseSecondaryMap<mir::LocalId, R8Id>,
    r16s: SparseSecondaryMap<mir::LocalId, (R8Id, R8Id)>,
    phi_source_r1s: SparseSecondaryMap<BlockId, IndexMap<mir::LocalId, R8Id>>,
    phi_source_r8s: SparseSecondaryMap<BlockId, IndexMap<mir::LocalId, R8Id>>,
    phi_source_r16s: SparseSecondaryMap<BlockId, IndexMap<mir::LocalId, (R8Id, R8Id)>>,
    phi_r1s: IndexMap<mir::LocalId, (R8Id, Vec<BlockId>)>,
    phi_r8s: IndexMap<mir::LocalId, (R8Id, Vec<BlockId>)>,
    phi_r16s: IndexMap<mir::LocalId, (R8Id, R8Id, Vec<BlockId>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum R1 {
    /// R1 represented by a zero flag
    Zero(ZeroId),
    /// R1 represented by a carry flag
    Carry(CarryId),
    /// R1 represented by a R8, with 0xFF as 1 and 0x00 as 0
    R8(R8Id),
    /// R1 represented by a bit set in a register
    Bit(R8Id, u3),
}
