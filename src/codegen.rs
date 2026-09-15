use crate::ast::Identifier;
use crate::lir::{
    BlockId, CarryId, CodeSectionBuilder, DataSectionData, FlagId, FnAddr, HLId, Ld16Source, Lir,
    LirBuilder, Operand8, R8Id, RamRegionKind, RamSectionData, RomRegionKind, SectionAddr,
    SectionId, Terminator, ZeroId,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::num::{BitCount, u3};
use crate::ty::{Ty, TyRegistry};
use crate::{attr, mir};
use indexmap::IndexMap;
use indexmap::map::Entry;
use miette::SourceSpan;
use slotmap::{SecondaryMap, SparseSecondaryMap};

pub const CART_HEADER_START_ADDR: u16 = 0x0100;
pub const CART_HEADER_END_ADDR: u16 = 0x014F;

/// Size in bytes of each entry in the RST/interrupt jump vector table.
///
/// See <https://gbdev.io/pandocs/Memory_Map.html#jump-vectors-in-first-rom-bank>
const VECTOR_SLOT_LEN: u16 = 8;

/// `RST` vector addresses, each handled with a bare `RET` stub.
const RST_VECTOR_ADDRS: [(u16, &str); 8] = [
    (0x0000, "@RST_00"),
    (0x0008, "@RST_08"),
    (0x0010, "@RST_10"),
    (0x0018, "@RST_18"),
    (0x0020, "@RST_20"),
    (0x0028, "@RST_28"),
    (0x0030, "@RST_30"),
    (0x0038, "@RST_38"),
];

/// Interrupt vector addresses, each handled with a bare `RETI` stub.
const INTERRUPT_VECTOR_ADDRS: [(u16, &str); 5] = [
    (0x0040, "@INT_VBLANK"),
    (0x0048, "@INT_LCD_STAT"),
    (0x0050, "@INT_TIMER"),
    (0x0058, "@INT_SERIAL"),
    (0x0060, "@INT_JOYPAD"),
];

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

        // code_section_id is used for the shared code section
        // functions with explicit addresses will be tracked in
        // their own code sections within pinned
        let code_section_id = self.lir.reserve_code_section();
        let mut code_section = CodeSectionBuilder::default();
        let mut functions = SecondaryMap::new();
        let mut pinned: Vec<(mir::FunctionId, SectionId, CodeSectionBuilder)> = Vec::new();
        for (function_id, function) in &mir.functions {
            if function.attrs.address.is_some()
                || function.attrs.region != attr::RomRegionKind::Auto
            {
                let section_id = self.lir.reserve_code_section();
                let mut section = CodeSectionBuilder::default();
                let region_kind =
                    self.lower_fn_rom_region(function.attrs.region, function.attrs.address);
                match function.attrs.address {
                    Some(addr) => section.set_region_and_addr(region_kind, addr),
                    None => section.set_region(region_kind),
                }

                let block = section.new_function(function.name.clone());
                functions.insert(
                    function_id,
                    FnAddr {
                        section: Some(section_id),
                        entry_block: block,
                    },
                );
                pinned.push((function_id, section_id, section));
            } else {
                let block = code_section.new_function(function.name.clone());
                functions.insert(
                    function_id,
                    FnAddr {
                        section: Some(code_section_id),
                        entry_block: block,
                    },
                );
            }
            if let Some(_addr) = function.attrs.address {}
        }
        for (function_id, function) in &mir.functions {
            if function.attrs.address.is_none() {
                self.generate_from_function(
                    mir,
                    function,
                    functions[function_id].entry_block,
                    &globals,
                    &functions,
                    &mut code_section,
                );
            }
        }
        self.lir.seal_code_section(code_section_id, code_section);

        for (function_id, section_id, mut section) in pinned {
            let function = &mir.functions[function_id];
            self.generate_from_function(
                mir,
                function,
                functions[function_id].entry_block,
                &globals,
                &functions,
                &mut section,
            );
            self.lir.seal_code_section(section_id, section);
        }

        self.generate_startup_code(mir, &functions);
    }

    /// Generates the startup code sections.
    ///
    /// This includes:
    ///
    /// * **RST vector stubs** `ROM0[0x0000..=0x003F]` - One `RET` stub per `RST` vector, so the
    ///   linker cannot place other code in the region until `RST` is supported
    /// * **Interrupt vector stubs** `ROM0[0x0040..=0x0067]` - One `RETI` stub per interrupt
    ///   vector, so the linker cannot place other code in the region until interrupts are
    ///   supported
    /// * **Cartridge header code section** `ROM0[0x0100..=0x0103]` - Jumps to the bootstrap code
    /// * **Cartridge header data section** `ROM0[0x0104..=0x014F]` - Zero'd out ROM data for the header data
    /// * **Bootstrap code section** `ROM0[auto]` - Initializes SP, calls to entrypoint, then spins
    fn generate_startup_code(
        &mut self,
        mir: &mir::Mir,
        functions: &SecondaryMap<mir::FunctionId, FnAddr>,
    ) {
        for &(addr, name) in &RST_VECTOR_ADDRS {
            self.generate_vector_stub(addr, name, Terminator::Ret);
        }
        for &(addr, name) in &INTERRUPT_VECTOR_ADDRS {
            self.generate_vector_stub(addr, name, Terminator::Reti);
        }

        const CART_HEADER_DATA_START_ADDR: u16 = CART_HEADER_START_ADDR + 4;
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
        let entrypoint_addr = functions[entrypoint];

        bootstrap_code.insert_ld_sp_value(bootstrap_code_block, Ld16Source::Literal(0xFFFE));
        let spin_block = bootstrap_code.new_block();
        bootstrap_code.set_terminator(
            bootstrap_code_block,
            Terminator::Call {
                callee: entrypoint_addr,
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

    /// Generates a single jump-vector stub: a 1-byte terminator instruction (`RET` or `RETI`)
    /// pinned at `addr`, followed by a zero'd out data section padding the rest of the
    /// [`VECTOR_SLOT_LEN`]-byte slot. Reserving the whole slot keeps the linker from placing
    /// other code in it before `RST`/interrupts are actually supported.
    fn generate_vector_stub(&mut self, addr: u16, name: &str, terminator: Terminator) {
        let code_section_id = self.lir.reserve_code_section();
        let mut code = CodeSectionBuilder::default();
        code.set_region_and_addr(RomRegionKind::Rom0, addr);
        let block = code.new_function(Identifier::from(name));
        code.set_terminator(block, terminator);
        self.lir.seal_code_section(code_section_id, code);

        let _ = self.lir.insert_data_section(DataSectionData {
            name: Identifier::from(format!("{name}_PAD")),
            region: Some(RomRegionKind::Rom0),
            addr: Some(addr + 1),
            data: vec![0; (VECTOR_SLOT_LEN - 1) as usize],
        });
    }

    fn generate_from_global(&mut self, global: &mir::Global) -> LoweredGlobal {
        match &global.attrs {
            mir::GlobalAttributes::Rom(attrs) => {
                LoweredGlobal::Section(self.lir.insert_data_section(DataSectionData {
                    name: global.name.data.clone(),
                    region: match attrs.region {
                        attr::RomRegionKind::Auto => None,
                        attr::RomRegionKind::RomX => Some(RomRegionKind::RomX(None)),
                        attr::RomRegionKind::Rom0 => Some(RomRegionKind::Rom0),
                    },
                    addr: attrs.address,
                    data: global.value.clone(),
                }))
            }
            mir::GlobalAttributes::Ram(attrs) => {
                let region = match attrs.region {
                    attr::RamRegionKind::Auto => {
                        if let Some(addr) = attrs.address {
                            match addr {
                                // https://gbdev.io/pandocs/Memory_Map.html
                                0x8000..=0x9FFF => RamRegionKind::VRam,
                                0xA000..=0xBFFF => RamRegionKind::SRam,
                                0xC000..=0xCFFF => RamRegionKind::WRam0,
                                0xD000..=0xDFFF => RamRegionKind::WRamX(None),
                                0xFE00..=0xFE9F => RamRegionKind::Oam,
                                0xFF00..=0xFF7F | 0xFFFF => return LoweredGlobal::Addr(addr),
                                0xFF80..=0xFFFE => RamRegionKind::HRam,
                                _ => RamRegionKind::default(),
                            }
                        } else {
                            RamRegionKind::default()
                        }
                    }
                    attr::RamRegionKind::WRam0 => RamRegionKind::WRam0,
                    attr::RamRegionKind::VRam => RamRegionKind::VRam,
                    attr::RamRegionKind::HRam => RamRegionKind::HRam,
                    attr::RamRegionKind::WRamX => RamRegionKind::WRamX(None),
                    attr::RamRegionKind::SRam => RamRegionKind::SRam,
                    attr::RamRegionKind::Oam => RamRegionKind::Oam,
                };
                if let Some(addr) = attrs.address {
                    match (region, addr) {
                        (RamRegionKind::VRam, 0x8000..=0x9FFF) => {}
                        (RamRegionKind::SRam, 0xA000..=0xBFFF) => {}
                        (RamRegionKind::WRam0, 0xC000..=0xCFFF) => {}
                        (RamRegionKind::WRamX(_), 0xD000..=0xDFFF) => {}
                        (RamRegionKind::Oam, 0xFE00..=0xFE9F) => {}
                        (RamRegionKind::HRam, 0xFF80..=0xFFFE) => {}
                        _ => self.result_builder.add_error(BadRamAddrError {
                            span: global.name.span,
                            name: global.name.data.clone(),
                            addr,
                        }),
                    }
                }
                LoweredGlobal::Section(self.lir.insert_ram_section(RamSectionData {
                    name: global.name.data.clone(),
                    size: global.ty.bit_size(&self.ty_registry),
                    region,
                    addr: attrs.address,
                }))
            }
        }
    }

    fn generate_from_function(
        &mut self,
        mir: &mir::Mir,
        function: &mir::Function,
        start_block: BlockId,
        globals: &SecondaryMap<mir::GlobalId, LoweredGlobal>,
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
                        let a = lir.insert_ld_a(block, source);
                        lir.insert_cpl(block, a);
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
                let (source, condition) = self.r1_to_flag(source, condition, block, lir);
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
                let (source, condition) = self.r1_to_flag(source, condition, block, lir);
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
                let (source, condition) = self.r1_to_flag(source, condition, block, lir);
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
                        let mask = 1 << u8::from(bit);
                        let existing = lir.insert_ld8_hl_ptr(block, target);
                        let a = lir.insert_ld_a(block, existing);
                        let (_z, _c) = lir.insert_and(block, a, Operand8::Literal(!mask));
                        // `source` is a full-byte 0x00/0xFF boolean, not just the target bit, so
                        // we need to move the cleared existing byte out of the accumulator and
                        // mask `source` down to that single bit before merging the two
                        let existing = lir.insert_move_a(block, a);
                        let a = lir.insert_ld_a(block, source);
                        if condition != mir::Condition::NotZero {
                            lir.insert_cpl(block, a);
                        }
                        let (_z, _c) = lir.insert_and(block, a, Operand8::Literal(mask));
                        let (_z, _c) = lir.insert_or(block, a, Operand8::Reg(existing));
                        // this move is unnecessary, but we'll rely on register coalescing
                        // to fix it
                        let source = lir.insert_move_a(block, a);
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
                let a = lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    lir.insert_cpl(block, a);
                }
                let (_z, _c) = if right_condition == mir::Condition::NotZero {
                    lir.insert_and(block, a, right)
                } else {
                    lir.insert_or(block, a, right)
                };

                let r8 = lir.insert_move_a(block, a);
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
                let a = lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    lir.insert_cpl(block, a);
                }
                let (_z, _c) = if right_condition == mir::Condition::NotZero {
                    lir.insert_or(block, a, right)
                } else {
                    lir.insert_and(block, a, right)
                };

                let r8 = lir.insert_move_a(block, a);
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
                let a = lir.insert_ld_a(block, left);
                let (_z, _c) = lir.insert_xor(block, a, right);
                if left_condition != right_condition {
                    lir.insert_cpl(block, a);
                }
                let r8 = lir.insert_move_a(block, a);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
        })
    }

    fn r1_to_flag(
        &mut self,
        r1: R1,
        condition: mir::Condition,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
    ) -> (FlagId, mir::Condition) {
        match r1 {
            R1::Zero(flag) => (FlagId::from(flag), condition),
            R1::Carry(flag) => (FlagId::from(flag), condition),
            R1::R8(r8) => {
                let r8 = lir.insert_ld_a(block, r8);
                let (z, _c) = lir.insert_cp(block, r8, Operand8::Literal(0xFF));
                (FlagId::from(z), condition)
            }
            R1::Bit(r8, bit) => {
                // `BIT` sets Z when the tested bit is 0, i.e. Z is SET for a *false* u1 value -
                // the opposite polarity to what the value represents so we need to invert the
                // condition
                (FlagId::from(lir.insert_bit(block, bit, r8)), !condition)
            }
        }
    }

    fn r1_to_r8(&mut self, r1: R1, block: BlockId, lir: &mut CodeSectionBuilder) -> R8Id {
        match r1 {
            R1::Carry(c_in) => {
                let a = lir.insert_fresh_a(block);
                let (_z, _c) = lir.insert_sbc_a_a(block, a, c_in);

                lir.insert_move_a(block, a)
            }
            R1::Zero(z_in) => {
                // zero flags are difficult, unfortunately. We need to push them onto
                // the stack then pop onto a 16-bit register
                lir.insert_push_af_z(block, z_in);
                let r16 = lir.insert_pop_r16(block);
                let (_high, low) = lir.insert_split(block, r16);
                // low now has the zero flag stored in the 7th bit. We can now
                // use the rotate left strategy
                let (_z, c_in) = lir.insert_rlc(block, low);
                let a = lir.insert_fresh_a(block);
                let (_z, _c) = lir.insert_sbc_a_a(block, a, c_in);
                lir.insert_move_a(block, a)
            }
            R1::R8(r8) => r8,
            R1::Bit(r8, bit) => {
                match u8::from(bit) {
                    0 => {
                        // rotate right essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let r8 = lir.insert_ld8(block, r8);
                        let (_z, c_in) = lir.insert_rrc(block, r8);
                        let a = lir.insert_fresh_a(block);
                        let (_z, _c) = lir.insert_sbc_a_a(block, a, c_in);
                        lir.insert_move_a(block, a)
                    }
                    7 => {
                        // rotate left essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let r8 = lir.insert_ld8(block, r8);
                        let (_z, c_in) = lir.insert_rlc(block, r8);
                        let a = lir.insert_fresh_a(block);
                        let (_z, _c) = lir.insert_sbc_a_a(block, a, c_in);
                        lir.insert_move_a(block, a)
                    }
                    _ => {
                        let mask = 1 << u8::from(bit);
                        // AND with the mask sets all the undefined bits to 0
                        let a = lir.insert_ld_a(block, r8);
                        let (_z, _c) = lir.insert_and(block, a, Operand8::Literal(mask));
                        // ADD !mask + 1 will move the bit into the carry bit
                        let (_z, c_in) = lir.insert_add(block, a, Operand8::Literal((!mask) + 1));
                        // now we can use the same trick as before
                        let (_z, _c) = lir.insert_sbc_a_a(block, a, c_in);
                        lir.insert_move_a(block, a)
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
                let a = lir.insert_ld_a(block, locals.r8s[*source]);
                lir.insert_cpl(block, a);
                let r8 = lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Neg { target, source } => {
                let a = lir.insert_ld_a(block, locals.r8s[*source]);
                lir.insert_cpl(block, a);
                let r8 = lir.insert_move_a(block, a);
                let _z = lir.insert_inc(block, r8);
                (*target, r8)
            }
            mir::Instr::RotateLeft { target, source } => {
                let r8 = lir.insert_ld8(block, locals.r8s[*source]);
                let (_z, _c) = lir.insert_rlc(block, r8);
                (*target, r8)
            }
            mir::Instr::RotateRight { target, source } => {
                let r8 = lir.insert_ld8(block, locals.r8s[*source]);
                let (_z, _c) = lir.insert_rrc(block, r8);
                (*target, r8)
            }
            mir::Instr::ShiftLeft { target, source } => {
                let r8 = lir.insert_ld8(block, locals.r8s[*source]);
                let (_z, _c) = lir.insert_sla(block, r8);
                (*target, r8)
            }
            mir::Instr::ShiftRight { target, source } => {
                let r8 = lir.insert_ld8(block, locals.r8s[*source]);
                let (_z, _c) = if ctx.function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, r8)
                } else {
                    lir.insert_srl(block, r8)
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
                let a = lir.insert_ld_a(block, locals.r8s[*left]);
                let (_z, _c) = lir.insert_and(block, a, right);
                let r8 = lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let a = lir.insert_ld_a(block, locals.r8s[*left]);
                let (_z, _c) = lir.insert_or(block, a, right);
                let r8 = lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let a = lir.insert_ld_a(block, locals.r8s[*left]);
                let (_z, _c) = lir.insert_xor(block, a, right);
                let r8 = lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let a = lir.insert_ld_a(block, locals.r8s[*left]);
                let (_z, _c) = lir.insert_add(block, a, right);
                let r8 = lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right, locals);
                let a = lir.insert_ld_a(block, locals.r8s[*left]);
                let (_z, _c) = lir.insert_sub(block, a, right);
                let r8 = lir.insert_move_a(block, a);
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
                let low = lir.insert_ld8_inc(block, addr);
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
                        let [low, high] = value.to_le_bytes();
                        let low = lir.insert_ld8_value(block, low);
                        let high = lir.insert_ld8_value(block, high);
                        (high, low)
                    }
                };
                let low = lir.insert_ld_a(block, low);
                lir.insert_store_inc(block, addr, low);
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
                let a = lir.insert_ld_a(block, source_low);
                lir.insert_cpl(block, a);
                let target_low = lir.insert_move_a(block, a);
                let a = lir.insert_ld_a(block, source_high);
                lir.insert_cpl(block, a);
                let target_high = lir.insert_move_a(block, a);
                (*target, target_high, target_low)
            }
            mir::Instr::Neg { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let a = lir.insert_ld_a(block, source_low);
                lir.insert_cpl(block, a);
                let target_low = lir.insert_move_a(block, a);
                let a = lir.insert_ld_a(block, source_high);
                lir.insert_cpl(block, a);
                let target_high = lir.insert_move_a(block, a);
                let target16 = lir.insert_join(block, target_high, target_low);
                lir.insert_inc16(block, target16);
                let (target_high, target_low) = lir.insert_split(block, target16);
                (*target, target_high, target_low)
            }
            mir::Instr::RotateLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);

                let high_copy = lir.insert_ld8(block, source_high);
                let (_z, c) = lir.insert_sla(block, high_copy);

                let target_low = lir.insert_ld8(block, source_low);
                let (_z, c) = lir.insert_rl(block, target_low, c);

                let target_high = lir.insert_ld8(block, source_high);
                let (_z, _c) = lir.insert_rl(block, target_high, c);

                (*target, target_high, target_low)
            }
            mir::Instr::RotateRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);

                let low_copy = lir.insert_ld8(block, source_low);
                let (_z, c) = lir.insert_sra(block, low_copy);

                let target_high = lir.insert_ld8(block, source_high);
                let (_z, c) = lir.insert_rr(block, target_high, c);

                let target_low = lir.insert_ld8(block, source_low);
                let (_z, _c) = lir.insert_rr(block, target_low, c);

                (*target, target_high, target_low)
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);

                let target_low = lir.insert_ld8(block, source_low);
                let (_z, c) = lir.insert_sla(block, target_low);

                let target_high = lir.insert_ld8(block, source_high);
                let (_z, _c) = lir.insert_rl(block, target_high, c);

                (*target, target_high, target_low)
            }
            mir::Instr::ShiftRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_from_operand(&mir::Operand::Local(*source), block, lir, ctx, locals);
                let target_high = lir.insert_ld8(block, source_high);
                let (_z, c) = if ctx.function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, target_high)
                } else {
                    lir.insert_srl(block, target_high)
                };
                let target_low = lir.insert_ld8(block, source_low);
                let (_z, _c) = lir.insert_rr(block, target_low, c);
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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, _c) = lir.insert_and(block, a, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, a);

                let a = lir.insert_ld_a(block, left_high);
                let (_z, _c) = lir.insert_and(block, a, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, a);

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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, _c) = lir.insert_or(block, a, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, a);

                let a = lir.insert_ld_a(block, left_high);
                let (_z, _c) = lir.insert_or(block, a, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, a);

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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, _c) = lir.insert_xor(block, a, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, a);

                let a = lir.insert_ld_a(block, left_high);
                let (_z, _c) = lir.insert_xor(block, a, Operand8::Reg(right_high));
                let target_high = lir.insert_move_a(block, a);

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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, c_in) = lir.insert_add(block, a, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, a);

                let a = lir.insert_ld_a(block, left_high);
                let (_z, _c) = lir.insert_adc(block, a, Operand8::Reg(right_high), c_in);
                let target_high = lir.insert_move_a(block, a);

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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, c_in) = lir.insert_sub(block, a, Operand8::Reg(right_low));
                let target_low = lir.insert_move_a(block, a);

                let a = lir.insert_ld_a(block, left_high);
                let (_z, _c) = lir.insert_sbc(block, a, Operand8::Reg(right_high), c_in);
                let target_high = lir.insert_move_a(block, a);

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
                let [low, high] = value.to_le_bytes();
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

                let a = lir.insert_ld_a(block, left_high);
                let (_z, c_in) = lir.insert_sub(block, a, Operand8::Reg(right_high));

                let a = lir.insert_ld_a(block, left_low);
                let (z, _c) = lir.insert_sbc(block, a, Operand8::Reg(right_low), c_in);

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

                let a = lir.insert_ld_a(block, left_low);
                let (_z, c_in) = lir.insert_sub(block, a, Operand8::Reg(right_low));

                let a = lir.insert_ld_a(block, left_high);
                let (_z, c) = lir.insert_sbc(block, a, Operand8::Reg(right_high), c_in);

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
                let a = lir.insert_ld_a(block, locals.r8s[left]);
                lir.insert_cp(block, a, right)
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
                            let _c = lir.insert_add16(block, r16, offset);
                            r16
                        }
                        None => self.offset_global_to_hl(global, byte_offset, block, lir, ctx),
                    };
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    let item_byte_size = ty.bit_size(&self.ty_registry).padded_bytes();
                    bit_offset = BitCount::new(0);
                    for _ in 0..item_byte_size {
                        let _c = lir.insert_add16(block, addr, index);
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
                    let _c = lir.insert_add16(block, hl, offset);
                    hl
                } else {
                    hl
                }
            }
            None => self.offset_global_to_hl(global, byte_offset, block, lir, ctx),
        };
        (source, bit_offset)
    }

    fn offset_global_to_hl(
        &self,
        global: mir::GlobalId,
        byte_offset: u16,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ctx: &InstrCtx,
    ) -> HLId {
        lir.insert_ld_hl(
            block,
            match &ctx.globals[global] {
                LoweredGlobal::Section(section) => Ld16Source::SectionAddr(SectionAddr {
                    section: *section,
                    offset: byte_offset,
                }),
                LoweredGlobal::Addr(addr) => Ld16Source::Literal(
                    addr.checked_add(byte_offset)
                        .expect("projection offset overflowed addr"),
                ),
            },
        )
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
        let r8 = if local_ty.bit_size(&self.ty_registry).into_u32() <= 8 {
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

    /// Resolves a `#[address]`-pinned function's ROM region, inferring `Rom0` vs `RomX` from `addr`
    /// when `region` is left as `Auto`.
    fn lower_fn_rom_region(&self, region: attr::RomRegionKind, addr: Option<u16>) -> RomRegionKind {
        match region {
            attr::RomRegionKind::Rom0 => RomRegionKind::Rom0,
            attr::RomRegionKind::RomX => RomRegionKind::RomX(None),
            attr::RomRegionKind::Auto => {
                if let Some(addr) = addr
                    && addr >= 0x4000
                {
                    RomRegionKind::RomX(None)
                } else {
                    RomRegionKind::Rom0
                }
            }
        }
    }
}

struct InstrCtx<'a> {
    mir: &'a mir::Mir,
    function: &'a mir::Function,
    globals: &'a SecondaryMap<mir::GlobalId, LoweredGlobal>,
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

enum LoweredGlobal {
    Section(SectionId),
    Addr(u16),
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{name}` cannot be placed at address ${addr:04x}")]
#[diagnostic()]
pub struct BadRamAddrError {
    #[label("bad address here")]
    pub span: SourceSpan,
    pub name: Identifier,
    pub addr: u16,
}
