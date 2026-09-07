use crate::ast::Identifier;
use crate::attr::{RamSectionKind, RomSectionKind};
use crate::lir::{
    BlockId, CarryId, CodeSectionBuilder, DataSectionData, FlagId, FnAddr, HLId, Ld16Source, Lir,
    LirBuilder, Operand8, R8Id, RamRegionKind, RamSectionData, RomRegionKind, SIGN_MASK,
    SectionAddr, SectionId, Terminator, ZeroId,
};
use crate::mir;
use crate::multi::{MultiError, MultiResultBuilder};
use crate::num::{BitCount, u3};
use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use indexmap::map::Entry;
use slotmap::{SecondaryMap, SparseSecondaryMap};

pub struct CodeGen {
    ty_registry: TyRegistry,
    result_builder: MultiResultBuilder,
    lir: LirBuilder,
}

impl From<TyRegistry> for CodeGen {
    fn from(ty_registry: TyRegistry) -> Self {
        CodeGen {
            ty_registry,
            result_builder: MultiResultBuilder::default(),
            lir: LirBuilder::default(),
        }
    }
}

impl CodeGen {
    pub fn generate(mut self, mir: &mir::Mir) -> Result<Lir, MultiError> {
        self.generate_from_mir(mir);
        self.result_builder.take().finish(()).into_result()?;
        Ok(self.lir.build())
    }

    fn generate_from_mir(&mut self, mir: &mir::Mir) {
        let mut globals = SecondaryMap::new();
        for (global_id, global) in &mir.globals {
            globals.insert(global_id, self.generate_from_global(global));
        }
        let code_section_id = self.lir.reserve_code_section();
        let mut code_section = CodeSectionBuilder::default();
        let mut functions = SecondaryMap::new();
        for (function_id, function) in &mir.functions {
            let block = code_section.new_function(function.name.clone());
            functions.insert(
                function_id,
                FnAddr {
                    section: None,
                    entry_block: block,
                },
            );
        }
        for (function_id, function) in &mir.functions {
            self.generate_from_function(
                mir,
                function,
                functions[function_id].entry_block,
                &globals,
                &functions,
                &mut code_section,
            );
        }
        self.lir.seal_code_section(code_section_id, code_section);

        self.generate_startup_code(mir, &functions, code_section_id);
    }

    /// Generates the startup code sections.
    ///
    /// This includes:
    ///
    /// * **Cartridge header code section** `ROM0[0x0100..=0x0103]` - Jumps to the bootstrap code
    /// * **Cartridge header data section** `ROM0[0x0104..=0x014F]` - Zero'd out ROM data for the header data
    /// * **Bootstrap code section** `ROM0[auto]` - Initializes SP, calls to entrypoint, then spins
    fn generate_startup_code(
        &mut self,
        mir: &mir::Mir,
        functions: &SecondaryMap<mir::FunctionId, FnAddr>,
        code_section_id: SectionId,
    ) {
        const CART_HEADER_START_ADDR: u16 = 0x0100;
        const CART_HEADER_DATA_START_ADDR: u16 = 0x0104;
        const CART_HEADER_END_ADDR: u16 = 0x014F;
        const CART_HEADER_DATA_LEN: usize =
            CART_HEADER_END_ADDR as usize - CART_HEADER_DATA_START_ADDR as usize + 1;
        let header_code_section_id = self.lir.reserve_code_section();

        let _ = self.lir.insert_data_section(DataSectionData {
            name: Identifier::from("@CART_HEADER_DATA"),
            region: Some(RomRegionKind::Rom0),
            addr: Some(CART_HEADER_DATA_START_ADDR),
            data: vec![0; CART_HEADER_DATA_LEN],
        });

        let bootstrap_code_section_id = self.lir.reserve_code_section();
        let mut bootstrap_code = CodeSectionBuilder::default();
        bootstrap_code.set_region(RomRegionKind::Rom0);
        let bootstrap_code_block = bootstrap_code.new_function(Identifier::from("@BOOTSTRAP"));

        let mut header_code = CodeSectionBuilder::default();
        header_code.set_region_and_addr(RomRegionKind::Rom0, CART_HEADER_START_ADDR);
        let header_code_block = header_code.new_function(Identifier::from("@CART_HEADER_CODE"));
        // 1 byte opcode for nop
        header_code.insert_nop(header_code_block);
        // 3 byte opcode for JP
        header_code.set_terminator(
            header_code_block,
            Terminator::JpFn {
                addr: FnAddr {
                    section: Some(bootstrap_code_section_id),
                    entry_block: bootstrap_code_block,
                },
            },
        );
        self.lir
            .seal_code_section(header_code_section_id, header_code);

        let entrypoint = mir
            .entrypoint
            .expect("mir must have a resolved entrypoint function");
        let entrypoint_block = functions[entrypoint].entry_block;

        bootstrap_code.insert_ld_sp_value(bootstrap_code_block, Ld16Source::Literal(0xFFFE));
        let spin_block = bootstrap_code.new_block();
        bootstrap_code.set_terminator(
            bootstrap_code_block,
            Terminator::Call {
                callee: FnAddr {
                    section: Some(code_section_id),
                    entry_block: entrypoint_block,
                },
                return_block: spin_block,
            },
        );
        bootstrap_code.set_terminator(
            spin_block,
            Terminator::Jp {
                next_block: spin_block,
            },
        );
        self.lir
            .seal_code_section(bootstrap_code_section_id, bootstrap_code);
    }

    fn generate_from_global(&mut self, global: &mir::Global) -> SectionId {
        match &global.attrs {
            mir::GlobalAttributes::Rom(attrs) => self.lir.insert_data_section(DataSectionData {
                name: global.name.data.clone(),
                region: match attrs.section {
                    RomSectionKind::Auto => None,
                    RomSectionKind::RomX => Some(RomRegionKind::RomX(None)),
                    RomSectionKind::Rom0 => Some(RomRegionKind::Rom0),
                },
                addr: None,
                data: global.value.clone(),
            }),
            mir::GlobalAttributes::Ram(attrs) => self.lir.insert_ram_section(RamSectionData {
                name: global.name.data.clone(),
                size: global.ty.bit_size(&self.ty_registry),
                region: match attrs.section {
                    RamSectionKind::Auto => RamRegionKind::default(),
                    RamSectionKind::WRam0 => RamRegionKind::WRam0,
                    RamSectionKind::VRam => RamRegionKind::VRam,
                    RamSectionKind::HRam => RamRegionKind::HRam,
                    RamSectionKind::WRamX => RamRegionKind::WRamX(None),
                    RamSectionKind::SRam => RamRegionKind::SRam,
                    RamSectionKind::Oam => RamRegionKind::Oam,
                },
                addr: 0,
            }),
        }
    }

    fn generate_from_function(
        &mut self,
        mir: &mir::Mir,
        function: &mir::Function,
        start_block: BlockId,
        globals: &SecondaryMap<mir::GlobalId, SectionId>,
        functions: &SecondaryMap<mir::FunctionId, FnAddr>,
        lir: &mut CodeSectionBuilder,
    ) {
        let mut blocks = SecondaryMap::new();
        let mut locals = Locals::default();
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
        for mir_block in function.traverse_blocks() {
            for instruction in &function.blocks[mir_block].instructions {
                if let mir::Instr::Phi { target, sources } = instruction {
                    match function.locals[*target]
                        .ty
                        .bit_size(&self.ty_registry)
                        .into_u32()
                    {
                        1 => {
                            let r8 = lir.reserve_phi();
                            locals.phi_r1s.insert(*target, (r8, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = blocks[mir_block];
                                match locals
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
                                locals.phi_r1s[target].1.push(block);
                            }
                        }
                        8 => {
                            let r8 = lir.reserve_phi();
                            locals.phi_r8s.insert(*target, (r8, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = blocks[mir_block];
                                match locals
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
                                locals.phi_r8s[target].1.push(block);
                            }
                        }
                        16 => {
                            let low = lir.reserve_phi();
                            let high = lir.reserve_phi();
                            locals.phi_r16s.insert(*target, (high, low, Vec::new()));
                            for (mir_block, local) in sources {
                                let block = blocks[mir_block];
                                match locals
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
                                locals.phi_r16s[target].2.push(block);
                            }
                        }
                        _ => unimplemented!(
                            "{} cannot fit in a 8-bit or 16-bit register",
                            function.locals[*target].ty.display(&self.ty_registry)
                        ),
                    }
                }
            }
        }
        for mir_block in function.traverse_blocks() {
            let block = blocks[mir_block];
            for instruction in &function.blocks[mir_block].instructions {
                let ty = self.ty_from_instr(mir, function, instruction);
                self.generate_from_instr(
                    instruction,
                    ty,
                    block,
                    lir,
                    &InstrCtx {
                        mir,
                        function,
                        globals,
                        functions,
                        blocks: &blocks,
                    },
                    &mut locals,
                );
            }
            if let Some(phis) = locals.phi_source_r1s.get(block) {
                for (local, &r8) in phis {
                    let (r1, condition) = locals.r1s[*local];
                    let mut source = self.r1_to_r8(r1, block, lir);
                    if condition != mir::Condition::NotZero {
                        let mut a = lir.insert_ld_a(block, source);
                        a = lir.insert_cpl(block, a);
                        source = lir.insert_move_a(block, a);
                    }
                    lir.insert_phi_ld(block, r8, source);
                }
            }
            if let Some(phis) = locals.phi_source_r8s.get(block) {
                for (local, &r8) in phis {
                    lir.insert_phi_ld(block, r8, locals.r8s[*local]);
                }
            }
            if let Some(phis) = locals.phi_source_r16s.get(block) {
                for (local, &(target_high, target_low)) in phis {
                    let (source_high, source_low) = locals.r16s[*local];
                    lir.insert_phi_ld(block, target_low, source_low);
                    lir.insert_phi_ld(block, target_high, source_high);
                }
            }
            self.generate_from_terminator(
                &function.blocks[mir_block].terminator,
                block,
                lir,
                &InstrCtx {
                    mir,
                    function,
                    globals,
                    functions,
                    blocks: &blocks,
                },
                &locals,
            );
        }
        for (r8, blocks) in locals.phi_r1s.values() {
            lir.complete_phi(*r8, blocks.as_slice());
        }
        for (r8, blocks) in locals.phi_r8s.values() {
            lir.complete_phi(*r8, blocks.as_slice());
        }
        for (high, low, blocks) in locals.phi_r16s.values() {
            lir.complete_phi(*low, blocks.as_slice());
            lir.complete_phi(*high, blocks.as_slice());
        }
    }

    fn ty_from_instr(
        &mut self,
        mir: &mir::Mir,
        function: &mir::Function,
        instruction: &mir::Instr,
    ) -> Ty {
        match (instruction.target_local(), instruction) {
            (Some(target), _) => function.locals[target].ty,
            (
                None,
                mir::Instr::Store {
                    target,
                    projections,
                    ..
                },
            ) => {
                let mut ty = mir.globals[*target].ty;
                for projection in projections {
                    ty = match projection {
                        mir::PlaceProjection::Field(field) => {
                            self.ty_registry.struct_ty_data(ty).fields[field].ty
                        }
                        mir::PlaceProjection::Index(_) => {
                            self.ty_registry.array_ty_data(ty).item_ty
                        }
                    }
                }
                ty
            }
            (None, _) => unreachable!("{instruction:?}"),
        }
    }

    /// generate LIR from the MIR instruction.
    ///
    /// An additional move after every lowered MIR instruction to ensure that between
    /// A MIR instruction will never have a pre-colored source as this would mean that there
    /// is likely interference due to the restrictive machine instructions.
    /// Unnecessary moves will be coalesced during register allocation if not needed.
    fn generate_from_instr(
        &mut self,
        instruction: &mir::Instr,
        ty: Ty,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &mut Locals,
    ) {
        match ty {
            Ty::U1 => {
                if let Some((target, r1, condition)) =
                    self.generate_from_u1_instr(instruction, block, lir, ctx, locals)
                {
                    let r1 = match r1 {
                        R1::Zero(z_in) => R1::Zero(lir.insert_move_zero(block, z_in)),
                        R1::Carry(c_in) => R1::Carry(lir.insert_move_carry(block, c_in)),
                        R1::R8(source) => R1::R8(lir.insert_ld8(block, source)),
                        R1::Bit(source, bit) => R1::Bit(lir.insert_ld8(block, source), bit),
                    };
                    locals.r1s.insert(target, (r1, condition));
                }
            }
            Ty::U2 => todo!("{instruction:?} {:?}", Ty::U2),
            Ty::U4 => todo!("{instruction:?} {:?}", Ty::U4),
            Ty::U8 | Ty::I8 => {
                if let Some((target, r8)) =
                    self.generate_from_scalar8_instr(instruction, block, lir, ctx, locals)
                {
                    let r8 = lir.insert_ld8(block, r8);
                    locals.r8s.insert(target, r8);
                }
            }
            Ty::U16 | Ty::I16 => {
                if let Some((target, high, low)) =
                    self.generate_from_scalar16_instr(instruction, block, lir, ctx, locals)
                {
                    let low = lir.insert_ld8(block, low);
                    let high = lir.insert_ld8(block, high);
                    locals.r16s.insert(target, (high, low));
                }
            }
            ty if ty.is_struct() || ty.is_array() || ty.is_fn_ptr() => match instruction {
                mir::Instr::Alloc { .. }
                | mir::Instr::ReturnValue { .. }
                | mir::Instr::Phi { .. }
                | mir::Instr::Load { .. }
                | mir::Instr::Store { .. }
                | mir::Instr::Copy { .. }
                | mir::Instr::Read { .. }
                | mir::Instr::Write { .. }
                | mir::Instr::Eq { .. }
                | mir::Instr::Ne { .. } => match ty.bit_size(&self.ty_registry).into_u32() {
                    8 => {
                        if let Some((target, r8)) =
                            self.generate_from_scalar8_instr(instruction, block, lir, ctx, locals)
                        {
                            let r8 = lir.insert_ld8(block, r8);
                            locals.r8s.insert(target, r8);
                        }
                    }
                    16 => {
                        if let Some((target, high, low)) =
                            self.generate_from_scalar16_instr(instruction, block, lir, ctx, locals)
                        {
                            let low = lir.insert_ld8(block, low);
                            let high = lir.insert_ld8(block, high);
                            locals.r16s.insert(target, (high, low));
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

    fn generate_from_terminator(
        &mut self,
        terminator: &mir::Terminator,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) {
        match terminator {
            mir::Terminator::Return => {
                lir.set_terminator(block, Terminator::Ret);
            }
            mir::Terminator::Call {
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
            } => {
                let callee = ctx.functions[*callee];
                let return_block = ctx.blocks[*mir_return_block];
                lir.set_terminator(
                    block,
                    Terminator::Call {
                        callee,
                        return_block,
                    },
                );
            }
            mir::Terminator::Call {
                callee: mir::Callee::Local(callee),
                return_block: mir_return_block,
            } => {
                let (callee_high, callee_low) = locals.r16s[*callee];
                let callee = lir.insert_join(block, callee_high, callee_low);
                let return_block = ctx.blocks[*mir_return_block];
                lir.set_terminator(
                    block,
                    Terminator::CallR16 {
                        callee,
                        return_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                source: mir_source,
                condition: mir_condition,
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = locals.r1s[*mir_source];
                let source = self.r1_to_flag(source, block, lir);
                let callee = ctx.functions[*callee];
                let return_block = ctx.blocks[*mir_return_block];
                let else_block = ctx.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::CallIf {
                        source,
                        negate: *mir_condition != condition,
                        callee,
                        return_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                callee: mir::Callee::Local(_),
                ..
            } => unimplemented!(
                "{terminator:?} as it should be rewritten within the optimization phase"
            ),
            mir::Terminator::Jump {
                target_block: mir_target_block,
            } => {
                let target_block = ctx.blocks[*mir_target_block];
                lir.set_terminator(
                    block,
                    Terminator::Jp {
                        next_block: target_block,
                    },
                );
            }
            mir::Terminator::JumpIf {
                source: mir_source,
                condition: mir_condition,
                then_block: mir_then_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = locals.r1s[*mir_source];
                let source = self.r1_to_flag(source, block, lir);
                let then_block = ctx.blocks[*mir_then_block];
                let else_block = ctx.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::JpIf {
                        source,
                        negate: *mir_condition != condition,
                        then_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::ReturnIf {
                source: mir_source,
                condition: mir_condition,
                else_block: mir_else_block,
            } => {
                let (source, condition) = locals.r1s[*mir_source];
                let source = self.r1_to_flag(source, block, lir);
                let else_block = ctx.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::RetIf {
                        source,
                        negate: *mir_condition != condition,
                        else_block,
                    },
                );
            }
        }
    }

    #[must_use]
    fn generate_from_u1_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> Option<(mir::LocalId, R1, mir::Condition)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = lir.insert_ld8_value(block, 0);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::ReturnValue { target } => {
                let r8 = lir.insert_ld_return_8(block);
                let r8 = lir.insert_move_a(block, r8);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::Phi { target, .. } => (
                *target,
                R1::R8(locals.phi_r1s[target].0),
                mir::Condition::NotZero,
            ),
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit) = self.projected_global_to_addr(
                    *source,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                let r8 = lir.insert_ld8_hl_ptr(block, addr);
                (*target, R1::Bit(r8, bit), mir::Condition::NotZero)
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (target, bit) = self.projected_global_to_addr(
                    *target,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                match self.r1_to_operand(source, block, lir, locals) {
                    (Operand8::Reg(source), condition) => {
                        let existing = lir.insert_ld8_hl_ptr(block, target);
                        let existing = lir.insert_ld_a(block, existing);
                        let (existing, _z, _c) = lir.insert_and(
                            block,
                            existing,
                            Operand8::Literal(!(1 << u8::from(bit))),
                        );
                        let source = if condition == mir::Condition::NotZero {
                            let (source, _z, _c) =
                                lir.insert_or(block, existing, Operand8::Reg(source));
                            source
                        } else {
                            // we need to flip the bits of the source, but the accumulator is already
                            // being used by existing, so we need to move exiting to another register
                            // first
                            let existing = lir.insert_move_a(block, existing);
                            let mut source = lir.insert_ld_a(block, source);
                            source = lir.insert_cpl(block, source);
                            let (source, _z, _c) =
                                lir.insert_or(block, source, Operand8::Reg(existing));
                            source
                        };
                        // this move is unnecessary, but we'll rely on register coalescing
                        // to fix it
                        let source = lir.insert_move_a(block, source);
                        lir.insert_store_hl(block, target, source);
                    }
                    (Operand8::Literal(mut source), condition) => {
                        if condition != mir::Condition::NotZero {
                            source = !source;
                        }
                        if source != 0 {
                            lir.insert_set_ptr(block, bit, target);
                        } else {
                            lir.insert_res_ptr(block, bit, target);
                        }
                    }
                    unexpected => unreachable!(
                        "projected_global_to_addr and r1_to_operand should not produce {unexpected:?}"
                    ),
                }
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let (r8, condition) = match source {
                    mir::Operand::Local(source) => {
                        let (r1, condition) = locals.r1s[*source];
                        let r8 = self.r1_to_r8(r1, block, lir);
                        (r8, condition)
                    }
                    mir::Operand::Literal(source) => {
                        let value = if source.value == 0 { 0x00u8 } else { 0xffu8 };
                        let r8 = lir.insert_ld8_value(block, value);
                        (r8, mir::Condition::NotZero)
                    }
                    mir::Operand::FnPtr(_) => unimplemented!("Cannot copy a fn ptr to a r1"),
                };
                (*target, R1::R8(r8), condition)
            }
            mir::Instr::Read {
                target,
                source,
                projections,
            } => {
                let (r8, bit) = self.projected_local_to_r8(
                    *source,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                (*target, R1::Bit(r8, bit), mir::Condition::NotZero)
            }
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let (flag, condition) = locals.r1s[*source];
                (*target, flag, !condition)
            }
            mir::Instr::Neg { .. } => unimplemented!("neg unsupported on u1"),
            mir::Instr::RotateLeft { target, source }
            | mir::Instr::RotateRight { target, source } => {
                let (r1, condition) = locals.r1s[*source];
                let r8 = self.r1_to_r8(r1, block, lir);
                (*target, R1::R8(r8), condition)
            }
            mir::Instr::ShiftLeft { target, .. } | mir::Instr::ShiftRight { target, .. } => {
                let r8 = lir.insert_ld8_value(block, 0);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::Eq {
                target,
                left,
                right,
            } => {
                let (z, condition) =
                    self.eq_from_comparison_op(*left, right, block, lir, ctx, locals);
                (*target, R1::Zero(z), condition)
            }
            mir::Instr::Ne {
                target,
                left,
                right,
            } => {
                let (z, condition) =
                    self.eq_from_comparison_op(*left, right, block, lir, ctx, locals);
                (*target, R1::Zero(z), !condition)
            }
            mir::Instr::Lt {
                target,
                left,
                right,
            } => {
                let (c, condition) =
                    self.lt_from_comparison_op(*left, right, block, lir, ctx, locals);
                (*target, R1::Carry(c), condition)
            }
            mir::Instr::Le { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::Ge {
                target,
                left,
                right,
            } => {
                let (c, condition) =
                    self.lt_from_comparison_op(*left, right, block, lir, ctx, locals);
                (*target, R1::Carry(c), !condition)
            }
            mir::Instr::Gt { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block, lir, locals);
                let left = self.r1_to_r8(r1, block, lir);
                let mut left = lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    left = lir.insert_cpl(block, left);
                }
                let (r8, _z, _c) = if right_condition == mir::Condition::NotZero {
                    lir.insert_and(block, left, right)
                } else {
                    lir.insert_or(block, left, right)
                };

                let r8 = lir.insert_move_a(block, r8);
                (*target, R1::R8(r8), right_condition)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block, lir, locals);
                let left = self.r1_to_r8(r1, block, lir);
                let mut left = lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    left = lir.insert_cpl(block, left);
                }
                let (r8, _z, _c) = if right_condition == mir::Condition::NotZero {
                    lir.insert_or(block, left, right)
                } else {
                    lir.insert_and(block, left, right)
                };

                let r8 = lir.insert_move_a(block, r8);
                (*target, R1::R8(r8), right_condition)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            }
            | mir::Instr::Add {
                target,
                left,
                right,
            }
            | mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block, lir, locals);
                let left = self.r1_to_r8(r1, block, lir);
                let left = lir.insert_ld_a(block, left);
                let (mut r8, _z, _c) = lir.insert_xor(block, left, right);
                if left_condition != right_condition {
                    r8 = lir.insert_cpl(block, r8);
                }
                let r8 = lir.insert_move_a(block, r8);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
        })
    }

    fn r1_to_flag(&mut self, r1: R1, block: BlockId, lir: &mut CodeSectionBuilder) -> FlagId {
        match r1 {
            R1::Zero(flag) => FlagId::from(flag),
            R1::Carry(flag) => FlagId::from(flag),
            R1::R8(r8) => {
                let r8 = lir.insert_ld_a(block, r8);
                let (z, _c) = lir.insert_cp(block, r8, Operand8::Literal(0xFF));
                FlagId::from(z)
            }
            R1::Bit(r8, bit) => FlagId::from(lir.insert_bit(block, bit, r8)),
        }
    }

    fn r1_to_r8(&mut self, r1: R1, block: BlockId, lir: &mut CodeSectionBuilder) -> R8Id {
        match r1 {
            R1::Carry(c_in) => {
                let (r8, _z, _c) = lir.insert_sbc_a_a(block, c_in);

                lir.insert_move_a(block, r8)
            }
            R1::Zero(z_in) => {
                // zero flags are difficult, unfortunately. We need to push them onto
                // the stack then pop onto a 16-bit register
                lir.insert_push_af_z(block, z_in);
                let r16 = lir.insert_pop_r16(block);
                let (_high, low) = lir.insert_split(block, r16);
                // low now has the zero flag stored in the 7th bit. We can now
                // use the rotate left strategy
                let (_r8, _z, c_in) = lir.insert_rlc(block, low);
                let (r8, _z, _c) = lir.insert_sbc_a_a(block, c_in);
                lir.insert_move_a(block, r8)
            }
            R1::R8(r8) => r8,
            R1::Bit(r8, bit) => {
                match u8::from(bit) {
                    0 => {
                        // rotate right essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let (_target, _z, c_in) = lir.insert_rrc(block, r8);
                        let (r8, _z, _c) = lir.insert_sbc_a_a(block, c_in);
                        lir.insert_move_a(block, r8)
                    }
                    7 => {
                        // rotate left essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let (_r8, _z, c_in) = lir.insert_rlc(block, r8);
                        let (r8, _z, _c) = lir.insert_sbc_a_a(block, c_in);
                        lir.insert_move_a(block, r8)
                    }
                    _ => {
                        let mask = 1 << u8::from(bit);
                        // AND with the mask sets all the undefined bits to 0
                        let r8 = lir.insert_ld_a(block, r8);
                        let (r8, _z, _c) = lir.insert_and(block, r8, Operand8::Literal(mask));
                        // ADD !mask + 1 will move the bit into the carry bit
                        let (_r8, _z, c_in) =
                            lir.insert_add(block, r8, Operand8::Literal((!mask) + 1));
                        // now we can use the same trick as before
                        let (r8, _z, _c) = lir.insert_sbc_a_a(block, c_in);
                        lir.insert_move_a(block, r8)
                    }
                }
            }
        }
    }

    fn r1_to_operand(
        &mut self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (Operand8, mir::Condition) {
        match right {
            mir::Operand::Local(right) => {
                let (r1, condition) = locals.r1s[*right];
                let r8 = self.r1_to_r8(r1, block, lir);
                (Operand8::Reg(r8), condition)
            }
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => {
                let value = if right.value == 0 { 0x00u8 } else { 0xffu8 };
                (Operand8::Literal(value), mir::Condition::NotZero)
            }
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 1-bit instruction using fn ptr")
            }
        }
    }

    #[must_use]
    fn generate_from_scalar8_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> Option<(mir::LocalId, R8Id)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = lir.insert_ld8_value(block, 0);
                (*target, r8)
            }
            mir::Instr::ReturnValue { target } => {
                let r8 = lir.insert_ld_return_8(block);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::Phi { target, sources: _ } => (*target, locals.phi_r8s[target].0),
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit_offset) = self.projected_global_to_addr(
                    *source,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                assert_eq!(bit_offset, u3::new(0), "misaligned 8-bit ty");
                let r8 = lir.insert_ld8_hl_ptr(block, addr);
                (*target, r8)
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (target, bit_offset) = self.projected_global_to_addr(
                    *target,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                assert_eq!(bit_offset, u3::new(0), "misaligned 8-bit ty");
                match self.r8_to_operand(source, locals) {
                    Operand8::Reg(source) => {
                        lir.insert_store_hl(block, target, source);
                    }
                    Operand8::Literal(source) => {
                        lir.insert_store_value(block, target, source);
                    }
                    unexpected => unreachable!(
                        "projected_global_to_addr and r8_to_operand should not produce {unexpected:?}"
                    ),
                }
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let r8 = lir.insert_ld8_operand(block, self.r8_to_operand(source, locals));
                (*target, r8)
            }
            mir::Instr::Read {
                target,
                source,
                projections,
            } => {
                let (r8, u3) = self.projected_local_to_r8(
                    *source,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                assert_eq!(u3, u3::new(0), "misaligned 8-bit ty");
                (*target, r8)
            }
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let r8 = locals.r8s[*source];
                let r8 = lir.insert_ld_a(block, r8);
                let r8 = lir.insert_cpl(block, r8);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::Neg { target, source } => {
                let r8 = locals.r8s[*source];
                let r8 = lir.insert_ld_a(block, r8);
                let (r8, _z, _c) = lir.insert_xor(block, r8, Operand8::Literal(SIGN_MASK));
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::RotateLeft { target, source } => {
                let (r8, _z, _c) = lir.insert_rlc(block, locals.r8s[*source]);
                (*target, r8)
            }
            mir::Instr::RotateRight { target, source } => {
                let (r8, _z, _c) = lir.insert_rrc(block, locals.r8s[*source]);
                (*target, r8)
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (r8, _z, _c) = lir.insert_sla(block, locals.r8s[*source]);
                (*target, r8)
            }
            mir::Instr::ShiftRight { target, source } => {
                let (r8, _z, _c) = if ctx.function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, locals.r8s[*source])
                } else {
                    lir.insert_srl(block, locals.r8s[*source])
                };
                (*target, r8)
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Le { .. }
            | mir::Instr::Ge { .. }
            | mir::Instr::Gt { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let left = locals.r8s[*left];
                let left = lir.insert_ld_a(block, left);
                let (r8, _z, _c) = lir.insert_and(block, left, right);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let left = locals.r8s[*left];
                let left = lir.insert_ld_a(block, left);
                let (r8, _z, _c) = lir.insert_or(block, left, right);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let left = locals.r8s[*left];
                let left = lir.insert_ld_a(block, left);
                let (r8, _z, _c) = lir.insert_xor(block, left, right);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let left = locals.r8s[*left];
                let left = lir.insert_ld_a(block, left);
                let (r8, _z, _c) = lir.insert_add(block, left, right);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let left = locals.r8s[*left];
                let left = lir.insert_ld_a(block, left);
                let (r8, _z, _c) = lir.insert_sub(block, left, right);
                let r8 = lir.insert_move_a(block, r8);
                (*target, r8)
            }
        })
    }

    #[must_use]
    fn generate_from_scalar16_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> Option<(mir::LocalId, R8Id, R8Id)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let low = lir.insert_ld8_value(block, 0);
                let high = lir.insert_ld8_value(block, 0);
                (*target, high, low)
            }
            mir::Instr::ReturnValue { target } => {
                let r16 = lir.insert_ld_return_16(block);
                let (high, low) = lir.insert_split(block, r16);
                let low = lir.insert_ld8(block, low);
                let high = lir.insert_ld8(block, high);
                (*target, high, low)
            }
            mir::Instr::Phi { target, sources: _ } => {
                let (high, low, _) = &locals.phi_r16s[target];
                (*target, *high, *low)
            }
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit_offset) = self.projected_global_to_addr(
                    *source,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                assert_eq!(bit_offset, u3::new(0), "misaligned 16-bit ty");
                let (low, addr) = lir.insert_ld8_inc(block, addr);
                let low = lir.insert_move_a(block, low);
                let high = lir.insert_ld8_hl_ptr(block, addr);
                (*target, high, low)
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (addr, bit_offset) = self.projected_global_to_addr(
                    *target,
                    projections.as_slice(),
                    block,
                    lir,
                    ctx,
                    locals,
                );
                assert_eq!(bit_offset, u3::new(0), "misaligned 16-bit ty");
                let (high, low) = match source {
                    mir::Operand::Local(source) => locals.r16s[*source],
                    mir::Operand::FnPtr(source) => {
                        let fn_addr = ctx.functions[*source];
                        let r16 = lir.insert_ld16(block, Ld16Source::FnAddr(fn_addr));
                        lir.insert_split(block, r16)
                    }
                    mir::Operand::Literal(source) => {
                        #[allow(clippy::cast_possible_truncation)]
                        let value = source.value as u16;
                        let [high, low] = value.to_le_bytes();
                        let low = lir.insert_ld8_value(block, low);
                        let high = lir.insert_ld8_value(block, high);
                        (high, low)
                    }
                };
                let low = lir.insert_ld_a(block, low);
                let addr = lir.insert_store_inc(block, addr, low);
                lir.insert_store_hl(block, addr, high);
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(source, block, lir, ctx, locals);
                let target_high = lir.insert_ld8(block, source_high);
                let target_low = lir.insert_ld8(block, source_low);
                (*target, target_high, target_low)
            }
            mir::Instr::Read { .. } => {
                unimplemented!("{instruction:?} should not be produced for 16-bit target")
            }
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let source_low = lir.insert_ld_a(block, source_low);
                let target_low = lir.insert_cpl(block, source_low);
                let target_low = lir.insert_move_a(block, target_low);
                let source_high = lir.insert_ld_a(block, source_high);
                let target_high = lir.insert_cpl(block, source_high);
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, target_low)
            }
            mir::Instr::Neg { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let source_high = lir.insert_ld_a(block, source_high);
                let (target_high, _z, _c) =
                    lir.insert_xor(block, source_high, Operand8::Literal(SIGN_MASK));
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, source_low)
            }
            mir::Instr::RotateLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let (_r8, _z, c) = lir.insert_sla(block, source_high);
                let (target_low, _z, c) = lir.insert_rl(block, source_low, c);
                let (target_high, _z, _c) = lir.insert_rl(block, source_high, c);
                (*target, target_high, target_low)
            }
            mir::Instr::RotateRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let (_r8, _z, c) = lir.insert_sra(block, source_low);
                let (target_high, _z, c) = lir.insert_rr(block, source_high, c);
                let (target_low, _z, _c) = lir.insert_rr(block, source_low, c);
                (*target, target_high, target_low)
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let (target_low, _z, c) = lir.insert_sla(block, source_low);
                let (target_high, _z, _c) = lir.insert_rl(block, source_high, c);
                (*target, target_high, target_low)
            }
            mir::Instr::ShiftRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let (target_high, _z, c) = if ctx.function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, source_high)
                } else {
                    lir.insert_srl(block, source_high)
                };
                let (target_low, _z, _c) = lir.insert_rr(block, source_low, c);
                (*target, target_high, target_low)
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Le { .. }
            | mir::Instr::Ge { .. }
            | mir::Instr::Gt { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(*left), block, lir, ctx, locals);
                let left_low = lir.insert_ld_a(block, left_low);
                let (target_low, _z, _c) =
                    lir.insert_and(block, left_low, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, target_low);
                let left_high = lir.insert_ld_a(block, left_high);
                let (target_high, _z, _c) =
                    lir.insert_and(block, left_high, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, target_low)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(*left), block, lir, ctx, locals);
                let left_low = lir.insert_ld_a(block, left_low);
                let (target_low, _z, _c) = lir.insert_or(block, left_low, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, target_low);
                let left_high = lir.insert_ld_a(block, left_high);
                let (target_high, _z, _c) =
                    lir.insert_or(block, left_high, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, target_low)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(*left), block, lir, ctx, locals);
                let left_low = lir.insert_ld_a(block, left_low);
                let (target_low, _z, _c) =
                    lir.insert_xor(block, left_low, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, target_low);
                let left_high = lir.insert_ld_a(block, left_high);
                let (target_high, _z, _c) =
                    lir.insert_xor(block, left_high, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, target_low)
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(*left), block, lir, ctx, locals);
                let left_low = lir.insert_ld_a(block, left_low);
                let (target_low, _z, c_in) =
                    lir.insert_add(block, left_low, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, target_low);
                let left_high = lir.insert_ld_a(block, left_high);
                let (target_high, _z, _c) =
                    lir.insert_adc(block, left_high, Operand8::Reg(right_high), c_in);
                let target_high = lir.insert_move_a(block, target_high);
                (*target, target_high, target_low)
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(*left), block, lir, ctx, locals);
                let left_high = lir.insert_ld_a(block, left_high);
                let (target_high, _z, c_in) =
                    lir.insert_sub(block, left_high, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, target_high);
                let left_low = lir.insert_ld_a(block, left_low);
                let (target_low, _z, _c) =
                    lir.insert_sbc(block, left_low, Operand8::Reg(right_low), c_in);
                let target_low = lir.insert_move_a(block, target_low);
                (*target, target_high, target_low)
            }
        })
    }

    fn r8_to_operand(&self, right: &mir::Operand, locals: &Locals) -> Operand8 {
        match right {
            mir::Operand::Local(right) => Operand8::Reg(locals.r8s[*right]),
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => Operand8::Literal(right.value as u8),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 8-bit instruction using fn ptr")
            }
        }
    }

    fn r16_from_operand(
        &self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (R8Id, R8Id) {
        match right {
            mir::Operand::Local(right) => locals.r16s[*right],
            mir::Operand::Literal(right) => {
                #[allow(clippy::cast_possible_truncation)]
                let value = right.value as u16;
                let [high, low] = value.to_le_bytes();
                let low = lir.insert_ld8_value(block, low);
                let high = lir.insert_ld8_value(block, high);
                (high, low)
            }
            mir::Operand::FnPtr(function) => {
                let fn_addr = ctx.functions[*function];
                let r16 = lir.insert_ld16(block, Ld16Source::FnAddr(fn_addr));
                lir.insert_split(block, r16)
            }
        }
    }

    fn eq_from_comparison_op(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (ZeroId, mir::Condition) {
        match ctx.function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (z, _c) = self.cp_from_r8(left, right, block, lir, ctx, locals);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                let (z, _c) = self.cp_from_r8(left, right, block, lir, ctx, locals);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 16 => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(left), block, lir, ctx, locals);
                let left_high = lir.insert_ld_a(block, left_high);
                let (_r16, _z, c_in) = lir.insert_sub(block, left_high, Operand8::Reg(right_high));
                let left_low = lir.insert_ld_a(block, left_low);
                let (_r16, z, _c) = lir.insert_sbc(block, left_low, Operand8::Reg(right_low), c_in);
                (z, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    fn lt_from_comparison_op(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (CarryId, mir::Condition) {
        match ctx.function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (_z, c) = self.cp_from_r8(left, right, block, lir, ctx, locals);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                let (_z, c) = self.cp_from_r8(left, right, block, lir, ctx, locals);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 16 => {
                let (right_high, right_low) = self.r16_from_operand(right, block, lir, ctx, locals);
                let (left_high, left_low) =
                    self.r16_from_operand(&mir::Operand::Local(left), block, lir, ctx, locals);
                let left_high = lir.insert_ld_a(block, left_high);
                let (_r16, _z, c_in) = lir.insert_sub(block, left_high, Operand8::Reg(right_high));
                let left_low = lir.insert_ld_a(block, left_low);
                let (_r16, _z, c) = lir.insert_sbc(block, left_low, Operand8::Reg(right_low), c_in);
                (c, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    fn cp_from_r8(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (ZeroId, CarryId) {
        match ctx.function.locals[left].ty {
            Ty::U1 => todo!("{left:?} {right:?} u1"),
            Ty::U2 => todo!("{left:?} {right:?} u2"),
            Ty::U4 => todo!("{left:?} {right:?} u4"),
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                let right = self.r8_to_operand(right, locals);
                let left = lir.insert_ld_a(block, locals.r8s[left]);
                lir.insert_cp(block, left, right)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    fn projected_global_to_addr(
        &mut self,
        global: mir::GlobalId,
        projections: &[mir::PlaceProjection],
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (HLId, u3) {
        let mut bit_offset = BitCount::new(0);
        let mut r16 = None;
        let mut ty = ctx.mir.globals[global].ty;
        for project in projections {
            match project {
                mir::PlaceProjection::Field(field) => {
                    let field_data = &self.ty_registry.struct_ty_data(ty).fields[field];
                    ty = field_data.ty;
                    bit_offset += field_data.bit_offset;
                }
                mir::PlaceProjection::Index(mir::Operand::Literal(index)) => {
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    bit_offset += ty.bit_size(&self.ty_registry).padded() * index.value;
                }
                mir::PlaceProjection::Index(mir::Operand::Local(index)) => {
                    let (index_high, index_low) = locals.r16s[*index];
                    let index = lir.insert_join(block, index_high, index_low);
                    let byte_offset = bit_offset.bytes();
                    assert_eq!(
                        bit_offset.bits(),
                        u3::new(0),
                        "Cannot index array at unpadded offset"
                    );
                    let addr = match r16 {
                        Some(r16) => {
                            let offset = lir.insert_ld16(block, Ld16Source::Literal(byte_offset));
                            let (r16, _c) = lir.insert_add16(block, r16, offset);
                            r16
                        }
                        None => lir.insert_ld_hl(
                            block,
                            Ld16Source::SectionAddr(SectionAddr {
                                section: ctx.globals[global],
                                offset: byte_offset,
                            }),
                        ),
                    };
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    let item_byte_size = ty.bit_size(&self.ty_registry).padded_bytes();
                    bit_offset = BitCount::new(0);
                    for _ in 0..item_byte_size {
                        let (addr, _c) = lir.insert_add16(block, addr, index);
                        r16 = Some(addr);
                    }
                }
                mir::PlaceProjection::Index(mir::Operand::FnPtr(_)) => {
                    unimplemented!("fn ptr should not be possible to use for indexing an array")
                }
            }
        }
        let byte_offset = bit_offset.bytes();
        let bit_offset = bit_offset.bits();
        let source = match r16 {
            Some(hl) => {
                if byte_offset > 0 {
                    let offset = lir.insert_ld16(block, Ld16Source::Literal(byte_offset));
                    let (hl, _c) = lir.insert_add16(block, hl, offset);
                    hl
                } else {
                    hl
                }
            }
            None => lir.insert_ld_hl(
                block,
                Ld16Source::SectionAddr(SectionAddr {
                    section: ctx.globals[global],
                    offset: byte_offset,
                }),
            ),
        };
        (source, bit_offset)
    }

    fn projected_local_to_r8(
        &mut self,
        local: mir::LocalId,
        projections: &[mir::PlaceProjection],
        _block: BlockId,
        _lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx<'_>,
        locals: &Locals,
    ) -> (R8Id, u3) {
        let mut bit_offset = BitCount::new(0);
        let local_ty = ctx.function.locals[local].ty;
        let mut ty = local_ty;
        for projection in projections {
            match projection {
                mir::PlaceProjection::Field(field) => {
                    let field_data = &self.ty_registry.struct_ty_data(ty).fields[field];
                    ty = field_data.ty;
                    bit_offset += field_data.bit_offset;
                }
                mir::PlaceProjection::Index(mir::Operand::Literal(index)) => {
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    bit_offset += ty.bit_size(&self.ty_registry).padded() * index.value;
                }
                mir::PlaceProjection::Index(mir::Operand::Local(_)) => {
                    todo!("cannot lower {projection:?} on local")
                }
                mir::PlaceProjection::Index(mir::Operand::FnPtr(_)) => {
                    unimplemented!("fn ptr should not be possible to use for indexing an array")
                }
            }
        }
        let r8 = if ty.bit_size(&self.ty_registry).into_u32() <= 8 {
            assert_eq!(bit_offset.bytes(), 0);
            locals.r8s[local]
        } else {
            let (high, low) = locals.r16s[local];
            match bit_offset.bytes() {
                0 => low,
                1 => high,
                _ => unimplemented!("cannot support bit_offset {bit_offset} > 16-bit"),
            }
        };
        (r8, bit_offset.bits())
    }
}

struct InstrCtx<'a> {
    mir: &'a mir::Mir,
    function: &'a mir::Function,
    globals: &'a SecondaryMap<mir::GlobalId, SectionId>,
    functions: &'a SecondaryMap<mir::FunctionId, FnAddr>,
    blocks: &'a SecondaryMap<mir::BlockId, BlockId>,
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
