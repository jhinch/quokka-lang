use crate::ast::Identifier;
use crate::lir::{
    CodeSectionBuilder, DataSectionData, FnAddr, Ld16Source, Lir, LirBuilder, RamRegionKind,
    RamSectionData, RomRegionKind, SectionId, Terminator,
};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::ty::TyRegistry;
use crate::{attr, mir};
use miette::SourceSpan;
use slotmap::SecondaryMap;

pub mod encode;
pub mod interference;
mod lower;
pub mod opcode;
pub mod peephole;
pub mod regalloc;

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
                lower::InstrCtx::new(
                    &self.ty_registry,
                    mir,
                    function,
                    functions[function_id].entry_block,
                    &globals,
                    &functions,
                    &mut code_section,
                )
                .generate_from_function();
            }
        }
        self.lir.seal_code_section(code_section_id, code_section);

        for (function_id, section_id, mut section) in pinned {
            let function = &mir.functions[function_id];
            lower::InstrCtx::new(
                &self.ty_registry,
                mir,
                function,
                functions[function_id].entry_block,
                &globals,
                &functions,
                &mut section,
            )
            .generate_from_function();
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
