//! # Quokka Lang's Low Intermediate Representation (LIR)
//!
//! LIR ([`Lir`]) represents a Quokka program as a collection of control flow
//! graphs with infinite virtual registers with all Phi instructions eliminated.
//! The instructions ([`Instr`]) within a basic block ([`Block`]) represent concrete
//! machine level instructions unlike [`crate::mir::Mir`] which represents language
//! level instructions.
//!
//! Virtual registers in LIR can be colored and allocated to physical registers to
//! produce the final assembly code.

use crate::ast::Identifier;
use crate::num::{BitCount, u3};
use crate::reg::Color;
use quokka_lang_slotkeyset::{SlotKeySet, SparseSlotKeySet};
use slotmap::sparse_secondary::Entry;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap, new_key_type};
use std::assert_matches;
use std::fmt;
use std::num::NonZeroU16;

new_key_type! { pub struct SectionId; }
new_key_type! { pub struct BlockId; }
new_key_type! { pub struct ZeroId; }
new_key_type! { pub struct CarryId; }
new_key_type! { pub struct R8Id; }
new_key_type! { pub struct R16Id; }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From)]
pub enum FlagId {
    Zero(ZeroId),
    Carry(CarryId),
}

impl FlagId {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            FlagId::Zero(flag) => write!(f, "{}", flag.display(lir, section)),
            FlagId::Carry(flag) => write!(f, "{}", flag.display(lir, section)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct AId(R8Id);

impl AId {
    #[must_use]
    pub fn upcast_to_r8_without_move(self) -> R8Id {
        self.0
    }

    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        self.0.display(lir, section)
    }

    #[must_use]
    pub fn display_revised_a<'a>(
        r8_out: AId,
        r8_in: AId,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.r8_registers[r8_out.0].color,
                code.r8_registers[r8_in.0].color,
            ) {
                (Some(Color::A), Some(Color::A)) => write!(f, "A"),
                (Some(Color::A), _) => {
                    write!(f, "A(in={})", r8_in.display(lir, section))
                }
                (_, Some(Color::A)) => {
                    write!(f, "A(out={})", r8_out.display(lir, section))
                }
                (_, _) => {
                    write!(
                        f,
                        "A(out={},in={})",
                        r8_out.display(lir, section),
                        r8_in.display(lir, section)
                    )
                }
            }
        })
    }
}

impl R8Id {
    #[must_use]
    pub fn downcast_to_a_without_move(self) -> AId {
        AId(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct HLId(R16Id);

impl HLId {
    #[must_use]
    pub fn upcast_to_r16_without_move(self) -> R16Id {
        self.0
    }

    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        self.0.display(lir, section)
    }

    #[must_use]
    pub fn display_revised_hl<'a>(
        hl_out: HLId,
        hl_in: HLId,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.r16_registers[hl_out.0].color,
                code.r16_registers[hl_in.0].color,
            ) {
                (Some(Color::HL), Some(Color::HL)) => write!(f, "HL"),
                (Some(Color::HL), _) => {
                    write!(f, "HL(in={})", hl_in.display(lir, section))
                }
                (_, Some(Color::HL)) => {
                    write!(f, "HL(out={})", hl_out.display(lir, section))
                }
                (_, _) => {
                    write!(
                        f,
                        "HL(out={},in={})",
                        hl_out.display(lir, section),
                        hl_in.display(lir, section)
                    )
                }
            }
        })
    }
}

impl R16Id {
    #[must_use]
    pub fn downcast_to_hl_without_move(self) -> HLId {
        HLId(self)
    }
}

impl SectionId {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match &lir.sections[*self].data {
            SectionData::Code(_) => Err(fmt::Error),
            SectionData::Data(data) => write!(f, "{}", data.name),
            SectionData::Ram(data) => write!(f, "{}", data.name),
        })
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".bb{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

impl ZeroId {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.zero_registers[*self].color {
                Some(Color::Fz) => write!(f, "z"),
                Some(_) => Err(fmt::Error),
                None => write!(f, "z#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff),
            }
        })
    }

    #[must_use]
    pub fn display_zero<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.zero_registers[*self].color {
                Some(Color::Fz) => write!(f, "z"),
                _ => {
                    write!(f, "z(out={})", self.display(lir, section))
                }
            }
        })
    }

    #[must_use]
    pub fn display_revised_zero<'a>(
        flag_out: ZeroId,
        flag_in: ZeroId,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.zero_registers[flag_out].color,
                code.zero_registers[flag_in].color,
            ) {
                (Some(Color::Fz), Some(Color::Fz)) => write!(f, "z"),
                (Some(Color::Fz), _) => {
                    write!(f, "z(in={})", flag_in.display(lir, section))
                }
                (_, Some(Color::Fz)) => {
                    write!(f, "z(out={})", flag_out.display(lir, section))
                }
                (_, _) => {
                    write!(
                        f,
                        "z(out={},in={})",
                        flag_out.display(lir, section),
                        flag_in.display(lir, section)
                    )
                }
            }
        })
    }
}

impl CarryId {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.carry_registers[*self].color {
                Some(Color::Fc) => write!(f, "c"),
                Some(_) => Err(fmt::Error),
                None => write!(f, "c#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff),
            }
        })
    }

    #[must_use]
    pub fn display_carry<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.carry_registers[*self].color {
                Some(Color::Fc) => write!(f, "c"),
                _ => {
                    write!(f, "c(out={})", self.display(lir, section))
                }
            }
        })
    }

    #[must_use]
    pub fn display_revised_carry<'a>(
        flag_out: CarryId,
        flag_in: CarryId,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.carry_registers[flag_out].color,
                code.carry_registers[flag_in].color,
            ) {
                (Some(Color::Fc), Some(Color::Fc)) => write!(f, "z"),
                (Some(Color::Fc), _) => {
                    write!(f, "c(in={})", flag_in.display(lir, section))
                }
                (_, Some(Color::Fc)) => {
                    write!(f, "c(out={})", flag_out.display(lir, section))
                }
                (_, _) => {
                    write!(
                        f,
                        "c(out={},in={})",
                        flag_out.display(lir, section),
                        flag_in.display(lir, section)
                    )
                }
            }
        })
    }
}

impl R8Id {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.r8_registers[*self].color {
                Some(Color::A) => write!(f, "A"),
                Some(Color::B) => write!(f, "B"),
                Some(Color::C) => write!(f, "C"),
                Some(Color::D) => write!(f, "D"),
                Some(Color::E) => write!(f, "E"),
                Some(Color::H) => write!(f, "H"),
                Some(Color::L) => write!(f, "L"),
                Some(_) => Err(fmt::Error),
                None => write!(f, "r8#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff),
            }
        })
    }

    #[must_use]
    pub fn display_revised<'a>(
        r8_out: R8Id,
        r8_in: R8Id,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.r8_registers[r8_out].color,
                code.r8_registers[r8_in].color,
            ) {
                (Some(color_out), Some(color_in)) if color_out == color_in => {
                    write!(f, "{}", r8_out.display(lir, section))
                }
                (Some(_), _) => {
                    write!(
                        f,
                        "{}(in={})",
                        r8_out.display(lir, section),
                        r8_in.display(lir, section)
                    )
                }
                (_, Some(_)) => {
                    write!(
                        f,
                        "{}(out={})",
                        r8_in.display(lir, section),
                        r8_out.display(lir, section)
                    )
                }
                (_, _) => {
                    write!(
                        f,
                        "(out={},in={})",
                        r8_out.display(lir, section),
                        r8_in.display(lir, section)
                    )
                }
            }
        })
    }
}

impl R16Id {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match code.r16_registers[*self].color {
                Some(Color::AF) => write!(f, "AF"),
                Some(Color::BC) => write!(f, "BC"),
                Some(Color::DE) => write!(f, "DE"),
                Some(Color::HL) => write!(f, "HL"),
                Some(Color::SP) => write!(f, "SP"),
                Some(_) => Err(fmt::Error),
                None => write!(f, "r16#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff),
            }
        })
    }

    #[must_use]
    pub fn display_revised<'a>(
        r16_out: R16Id,
        r16_in: R16Id,
        lir: &'a Lir,
        section: SectionId,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let SectionData::Code(code) = &lir.sections[section].data else {
                return Err(fmt::Error);
            };
            match (
                code.r16_registers[r16_out].color,
                code.r16_registers[r16_in].color,
            ) {
                (Some(color_out), Some(color_in)) if color_out == color_in => {
                    write!(f, "{}", r16_out.display(lir, section))
                }
                (Some(_), _) => {
                    write!(
                        f,
                        "{}(in={})",
                        r16_out.display(lir, section),
                        r16_in.display(lir, section)
                    )
                }
                (_, Some(_)) => {
                    write!(
                        f,
                        "{}(out={})",
                        r16_in.display(lir, section),
                        r16_out.display(lir, section)
                    )
                }
                (_, _) => {
                    write!(
                        f,
                        "(out={},in={})",
                        r16_out.display(lir, section),
                        r16_in.display(lir, section)
                    )
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From)]
pub enum VirtualReg {
    R8(R8Id),
    R16(R16Id),
    Zero(ZeroId),
    Carry(CarryId),
}

impl From<AId> for VirtualReg {
    fn from(value: AId) -> Self {
        VirtualReg::R8(value.0)
    }
}

impl From<HLId> for VirtualReg {
    fn from(value: HLId) -> Self {
        VirtualReg::R16(value.0)
    }
}

pub const ZERO_FLAG_BIT: u8 = 7;
pub const ZERO_FLAG_MASK: u8 = 1 << ZERO_FLAG_BIT;
pub const CARRY_FLAG_BIT: u8 = 4;
pub const CARRY_FLAG_MASK: u8 = 1 << CARRY_FLAG_BIT;
pub const SIGN_BIT: u8 = 7;
pub const SIGN_MASK: u8 = 1 << SIGN_BIT;

#[derive(Debug, Default)]
pub struct Lir {
    pub sections: SlotMap<SectionId, Section>,
}

impl fmt::Display for Lir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (section_id, section) in &self.sections {
            writeln!(f, "{}", section.display(self, section_id))?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Section {
    pub data: SectionData,
}

impl Section {
    #[must_use]
    pub fn region(&self) -> RegionKind {
        match &self.data {
            SectionData::Code(CodeSectionData { region, .. })
            | SectionData::Data(DataSectionData { region, .. }) => {
                RegionKind::Rom(region.unwrap_or_default())
            }
            SectionData::Ram(RamSectionData { region, .. }) => RegionKind::Ram(*region),
        }
    }

    #[must_use]
    pub fn addr(&self) -> Option<u16> {
        match &self.data {
            SectionData::Code(CodeSectionData { addr, .. })
            | SectionData::Data(DataSectionData { addr, .. }) => *addr,
            SectionData::Ram(RamSectionData { addr, .. }) => Some(*addr),
        }
    }

    #[must_use]
    pub fn bank(&self) -> Option<NonZeroU16> {
        match &self.data {
            SectionData::Code(CodeSectionData { region, .. })
            | SectionData::Data(DataSectionData { region, .. }) => match region.as_ref()? {
                RomRegionKind::RomX(bank) => *bank,
                RomRegionKind::Rom0 => None,
            },
            SectionData::Ram(RamSectionData { region, .. }) => match region {
                RamRegionKind::WRamX(bank) => *bank,
                RamRegionKind::WRam0
                | RamRegionKind::VRam
                | RamRegionKind::HRam
                | RamRegionKind::SRam
                | RamRegionKind::Oam => None,
            },
        }
    }
}

impl Section {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            write!(f, "START_SECTION {}", self.region())?;
            if let Some(addr) = self.addr() {
                write!(f, "[{addr}]")?;
            }
            writeln!(f)?;
            match &self.data {
                SectionData::Code(data) => {
                    for (block_id, block) in &data.blocks {
                        if let Some(name) = data.functions.get(block_id) {
                            writeln!(f, "{name}::")?;
                        }
                        writeln!(f, "{block_id}:")?;
                        writeln!(f, "{}", block.display(lir, section))?;
                    }
                }
                SectionData::Data(data) => {
                    writeln!(f, "{}::", data.name)?;
                    for chunk in data.data.chunks(32) {
                        match chunk {
                            [] => {}
                            [byte, tail @ ..] => {
                                write!(f, "data ${byte:02x}")?;
                                for byte in tail {
                                    write!(f, ", ${byte:02x}")?;
                                }
                                writeln!(f)?;
                            }
                        }
                    }
                }
                SectionData::Ram(data) => {
                    writeln!(f, "{}::", data.name)?;
                    writeln!(f, "alloc {} x bit", data.size)?;
                }
            }
            writeln!(f, "END_SECTION")
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RegionKind {
    Rom(RomRegionKind),
    Ram(RamRegionKind),
}

impl fmt::Display for RegionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionKind::Rom(kind) => write!(f, "{kind}"),
            RegionKind::Ram(kind) => write!(f, "{kind}"),
        }
    }
}

#[derive(Debug)]
pub enum SectionData {
    Code(CodeSectionData),
    Data(DataSectionData),
    Ram(RamSectionData),
}

#[derive(Debug, Default)]
pub struct CodeSectionData {
    pub region: Option<RomRegionKind>,
    pub addr: Option<u16>,
    pub functions: SparseSecondaryMap<BlockId, Identifier>,
    pub blocks: SlotMap<BlockId, Block>,
    pub zero_registers: SlotMap<ZeroId, Register>,
    pub carry_registers: SlotMap<CarryId, Register>,
    pub r8_registers: SlotMap<R8Id, Register>,
    pub r16_registers: SlotMap<R16Id, Register>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum RomRegionKind {
    #[default]
    Rom0,
    RomX(Option<NonZeroU16>),
}

impl fmt::Display for RomRegionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomRegionKind::Rom0 => write!(f, "ROM0"),
            RomRegionKind::RomX(Some(bank)) => write!(f, "ROM{bank}"),
            RomRegionKind::RomX(None) => write!(f, "ROMX"),
        }
    }
}

#[derive(Debug)]
pub struct DataSectionData {
    pub name: Identifier,
    pub region: Option<RomRegionKind>,
    pub addr: Option<u16>,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct RamSectionData {
    pub name: Identifier,
    pub region: RamRegionKind,
    pub addr: u16,
    pub size: BitCount,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum RamRegionKind {
    #[default]
    WRam0,
    VRam,
    HRam,
    WRamX(Option<NonZeroU16>),
    SRam,
    Oam,
}

impl fmt::Display for RamRegionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RamRegionKind::WRam0 => write!(f, "WRAM0"),
            RamRegionKind::WRamX(Some(bank)) => write!(f, "WRAM{bank}"),
            RamRegionKind::WRamX(None) => write!(f, "WRAMX"),
            RamRegionKind::VRam => write!(f, "VRAM"),
            RamRegionKind::HRam => write!(f, "HRAM"),
            RamRegionKind::SRam => write!(f, "SRAM"),
            RamRegionKind::Oam => write!(f, "OAM"),
        }
    }
}

/// LIR function
#[derive(Debug)]
pub struct Function {
    pub name: Identifier,
    pub start_block: BlockId,
}

/// LIR Basic block
#[derive(Debug)]
pub struct Block {
    pub instructions: Vec<Instr>,
    pub terminator: Terminator,
    pub previous: Vec<BlockId>,
}

impl Block {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            for instruction in &self.instructions {
                writeln!(f, "  {}", instruction.display(lir, section))?;
            }
            writeln!(f, "  {}", self.terminator.display(lir, section))?;
            Ok(())
        })
    }

    pub fn next_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.terminator.next_blocks()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Register {
    pub defined: SparseSecondaryMap<BlockId, usize>,
    pub color: Option<Color>,
}

#[derive(Debug, Clone)]
pub enum Instr {
    /// `NOP`
    Nop,
    /// `LD r8,r8`
    Ld8 { target: R8Id, source: R8Id },
    /// `LD r8,n8`
    Ld8Value { target: R8Id, source: u8 },
    /// `LD r8,[HL]`
    /// `LD A,[r16]`
    /// `LD A,[n16]`
    Ld8Ptr { target: R8Id, source_addr: Addr },
    /// `LDH A,[n16]`
    /// `LDH A,[C]`
    Ld8HighPtr { target: AId, source_addr: HighAddr },
    /// `LD A,[HLI]`
    Ld8Inc {
        target: AId,
        revised_addr: HLId,
        source_addr: HLId,
    },
    /// `LD A,[HLD]`
    Ld8Dec {
        target: AId,
        revised_addr: HLId,
        source_addr: HLId,
    },
    /// `LD r16,n16`
    Ld16 { target: R16Id, source: Ld16Source },
    /// `LD [HL],r8`
    /// `LD [r16],A`
    /// `LD [n16],A`
    Store { target_addr: Addr, source: R8Id },
    /// `LDH [n16],A`
    /// `LDH [C],A`
    StoreHigh { addr: HighAddr, source: AId },
    /// `LD [HL],n8`
    StoreValue { addr: HLId, source: u8 },
    /// `LD [HLI],A`
    StoreInc {
        revised_addr: HLId,
        addr: HLId,
        source: AId,
    },
    /// `LD [HLD],A`
    StoreDec {
        revised_addr: HLId,
        addr: HLId,
        source: AId,
    },
    /// `ADC A,r8`
    /// `ADC A,[HL]`
    /// `ADC A,n8`
    Adc {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
        c_in: CarryId,
    },
    /// `ADD A,r8`
    /// `ADD A,[HL]`
    /// `ADD A,n8`
    Add {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `CP A,r8`
    /// `CP A,[HL]`
    /// `CP A,n8`
    Cp {
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `DEC r8`
    Dec {
        target: R8Id,
        z: ZeroId,
        source: R8Id,
    },
    /// `DEC [HL]`
    DecPtr { z: ZeroId, addr: HLId },
    /// `INC r8`
    Inc {
        target: R8Id,
        z: ZeroId,
        source: R8Id,
    },
    /// `INC [HL]`
    IncPtr { z: ZeroId, addr: HLId },
    /// `SBC A,r8`
    /// `SBC A,[HL]`
    /// `SBC A,n8`
    Sbc {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
        c_in: CarryId,
    },
    /// `SUB A,r8`
    /// `SUB A,[HL]`
    /// `SUB A,n8`
    Sub {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `AND A,r8`
    /// `AND A,[HL]`
    /// `AND A,n8`
    And {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `CPL`
    Cpl { target: AId, source: AId },
    /// `OR A,r8`
    /// `OR A,[HL]`
    /// `OR A,n8`
    Or {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `XOR A,r8`
    /// `XOR A,[HL]`
    /// `XOR A,n8`
    Xor {
        target: AId,
        z: ZeroId,
        c: CarryId,
        left: AId,
        right: Operand8,
    },
    /// `BIT u3,r8`
    Bit { z: ZeroId, bit: u3, source: R8Id },
    /// `BIT u3,[HL]`
    BitPtr {
        z: ZeroId,
        bit: u3,
        source_addr: HLId,
    },
    /// `RES u3,r8`
    Res { target: R8Id, bit: u3, source: R8Id },
    /// `RES u3,[HL]`
    ResPtr { bit: u3, addr: HLId },
    /// `SET u3,r8`
    Set { target: R8Id, bit: u3, source: R8Id },
    /// `SET u3,[HL]`
    SetPtr { bit: u3, addr: HLId },
    /// `RL r8`
    /// `RLA`
    Rl {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
        c_in: CarryId,
    },
    /// `RL [HL]`
    RlPtr {
        z: ZeroId,
        c: CarryId,
        addr: HLId,
        c_in: CarryId,
    },
    /// `RLC r8`
    /// `RLCA`
    Rlc {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `RLC [HL]`
    RlcPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `RR r8`
    /// `RRA`
    Rr {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
        c_in: CarryId,
    },
    /// `RR [HL]`
    RrPtr {
        z: ZeroId,
        c: CarryId,
        addr: HLId,
        c_in: CarryId,
    },
    /// `RRC r8`
    /// `RRCA`
    Rrc {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `RRC [HL]`
    RrcPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `SLA r8`
    Sla {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `SLA [HL]`
    SlaPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `SRA r8`
    Sra {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `SRA [HL]`
    SraPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `SRL r8`
    Srl {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `SRL [HL]`
    SrlPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `SWAP r8`
    Swap {
        target: R8Id,
        z: ZeroId,
        c: CarryId,
        source: R8Id,
    },
    /// `SWAP [HL]`
    SwapPtr { z: ZeroId, c: CarryId, addr: HLId },
    /// `CCF`
    Ccf { c: CarryId, c_in: CarryId },
    /// `SCF`
    Scf { c: CarryId },
    /// `ADD HL,r16`
    Add16 {
        target: HLId,
        c: CarryId,
        left: HLId,
        right: R16Id,
    },
    /// `ADD SP e8`
    AddSP { z: ZeroId, c: CarryId, right: i8 },
    /// `LD HL,SP+e8`
    LdSPE8 { target: HLId, c: CarryId, right: i8 },
    /// `DEC r16`
    Dec16 { target: R16Id, source: R16Id },
    /// `INC r16`
    Inc16 { target: R16Id, source: R16Id },
    /// `PUSH r16`
    PushR16 { source: R16Id },
    /// `PUSH AF`
    PushAF {
        source: AId,
        z_in: ZeroId,
        c_in: CarryId,
    },
    /// `POP r16`
    PopR16 { target: R16Id },
    /// `POP AF`
    PopAF { target: AId, z: ZeroId, c: CarryId },
    /// Virtual instruction to join 2x 8-bit registers into 1x 16-bit register.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    Join {
        target: R16Id,
        high_in: R8Id,
        low_in: R8Id,
    },
    /// Virtual instruction to split 1 x 16-bit register into 2x 8-bit registers.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    Split {
        high: R8Id,
        low: R8Id,
        source: R16Id,
    },
    /// Virtual instruction to move a zero flag
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to convert a precolored zero flag into an unconstrained flag.
    MoveZero { z: ZeroId, z_in: ZeroId },
    /// Virtual instruction to move a carry flag
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to convert a precolored carry flag into an unconstrained flag.
    MoveCarry { c: CarryId, c_in: CarryId },
}

impl Instr {
    #[must_use]
    pub fn is_move(&self) -> bool {
        matches!(
            self,
            Instr::Ld8 { .. } | Instr::MoveZero { .. } | Instr::MoveCarry { .. }
        )
    }

    #[must_use]
    pub fn as_move(&self) -> Option<(VirtualReg, VirtualReg)> {
        Some(match self {
            Instr::Ld8 { target, source } => (VirtualReg::R8(*target), VirtualReg::R8(*source)),
            Instr::MoveZero { z, z_in } => (VirtualReg::Zero(*z), VirtualReg::Zero(*z_in)),
            Instr::MoveCarry { c, c_in } => (VirtualReg::Carry(*c), VirtualReg::Carry(*c_in)),
            _ => return None,
        })
    }

    pub fn register_defs(&self) -> impl Iterator<Item = VirtualReg> + '_ {
        self.target_r8s()
            .map(VirtualReg::R8)
            .chain(self.target_r16().into_iter().map(VirtualReg::R16))
            .chain(self.zero().map(VirtualReg::Zero))
            .chain(self.carry().map(VirtualReg::Carry))
    }

    pub fn register_uses(&self) -> impl Iterator<Item = VirtualReg> + '_ {
        self.source_r8()
            .map(VirtualReg::R8)
            .chain(self.source_r16().map(VirtualReg::R16))
            .chain(self.zero_in().map(VirtualReg::Zero))
            .chain(self.carry_in().map(VirtualReg::Carry))
    }

    pub fn target_r8s(&self) -> impl Iterator<Item = R8Id> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let r8 = match self {
                Instr::Split { high, low, .. } => match next {
                    0 => Some(*low),
                    1 => Some(*high),
                    _ => None,
                },
                Instr::Ld8 { target, .. }
                | Instr::Ld8Value { target, .. }
                | Instr::Ld8Ptr { target, .. }
                | Instr::Dec { target, .. }
                | Instr::Inc { target, .. }
                | Instr::Res { target, .. }
                | Instr::Set { target, .. }
                | Instr::Rl { target, .. }
                | Instr::Rlc { target, .. }
                | Instr::Rr { target, .. }
                | Instr::Rrc { target, .. }
                | Instr::Sla { target, .. }
                | Instr::Sra { target, .. }
                | Instr::Srl { target, .. }
                | Instr::Swap { target, .. } => match next {
                    0 => Some(*target),
                    _ => None,
                },
                Instr::Ld8HighPtr { target, .. }
                | Instr::Ld8Inc { target, .. }
                | Instr::Ld8Dec { target, .. }
                | Instr::Adc { target, .. }
                | Instr::Add { target, .. }
                | Instr::Sbc { target, .. }
                | Instr::Sub { target, .. }
                | Instr::And { target, .. }
                | Instr::Cpl { target, .. }
                | Instr::Or { target, .. }
                | Instr::Xor { target, .. }
                | Instr::PopAF { target, .. } => match next {
                    0 => Some(target.0),
                    _ => None,
                },
                Instr::Nop
                | Instr::Ld16 { .. }
                | Instr::Store { .. }
                | Instr::StoreHigh { .. }
                | Instr::StoreValue { .. }
                | Instr::StoreInc { .. }
                | Instr::StoreDec { .. }
                | Instr::Cp { .. }
                | Instr::DecPtr { .. }
                | Instr::IncPtr { .. }
                | Instr::Bit { .. }
                | Instr::BitPtr { .. }
                | Instr::ResPtr { .. }
                | Instr::SetPtr { .. }
                | Instr::RlPtr { .. }
                | Instr::RlcPtr { .. }
                | Instr::RrPtr { .. }
                | Instr::RrcPtr { .. }
                | Instr::SlaPtr { .. }
                | Instr::SraPtr { .. }
                | Instr::SrlPtr { .. }
                | Instr::SwapPtr { .. }
                | Instr::Ccf { .. }
                | Instr::Scf { .. }
                | Instr::Add16 { .. }
                | Instr::Dec16 { .. }
                | Instr::Inc16 { .. }
                | Instr::Join { .. }
                | Instr::MoveZero { .. }
                | Instr::MoveCarry { .. }
                | Instr::AddSP { .. }
                | Instr::LdSPE8 { .. }
                | Instr::PushAF { .. }
                | Instr::PushR16 { .. }
                | Instr::PopR16 { .. } => None,
            }?;
            next += 1;
            Some(r8)
        })
    }

    #[must_use]
    pub fn target_r16(&self) -> Option<R16Id> {
        match self {
            Instr::Add16 { target, .. }
            | Instr::Ld8Inc {
                revised_addr: target,
                ..
            }
            | Instr::Ld8Dec {
                revised_addr: target,
                ..
            }
            | Instr::StoreInc {
                revised_addr: target,
                ..
            }
            | Instr::StoreDec {
                revised_addr: target,
                ..
            }
            | Instr::LdSPE8 { target, .. } => Some(target.0),
            Instr::Dec16 { target, .. }
            | Instr::Inc16 { target, .. }
            | Instr::Join { target, .. }
            | Instr::Ld16 { target, .. }
            | Instr::PopR16 { target, .. } => Some(*target),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Value { .. }
            | Instr::Ld8Ptr { .. }
            | Instr::Ld8HighPtr { .. }
            | Instr::Adc { .. }
            | Instr::Add { .. }
            | Instr::Cp { .. }
            | Instr::Dec { .. }
            | Instr::Inc { .. }
            | Instr::Sbc { .. }
            | Instr::Sub { .. }
            | Instr::And { .. }
            | Instr::Cpl { .. }
            | Instr::Or { .. }
            | Instr::Xor { .. }
            | Instr::Res { .. }
            | Instr::Set { .. }
            | Instr::Rl { .. }
            | Instr::Rlc { .. }
            | Instr::Rr { .. }
            | Instr::Rrc { .. }
            | Instr::Sla { .. }
            | Instr::Sra { .. }
            | Instr::Srl { .. }
            | Instr::Swap { .. }
            | Instr::Store { .. }
            | Instr::StoreHigh { .. }
            | Instr::StoreValue { .. }
            | Instr::DecPtr { .. }
            | Instr::IncPtr { .. }
            | Instr::Bit { .. }
            | Instr::BitPtr { .. }
            | Instr::ResPtr { .. }
            | Instr::SetPtr { .. }
            | Instr::RlPtr { .. }
            | Instr::RlcPtr { .. }
            | Instr::RrPtr { .. }
            | Instr::RrcPtr { .. }
            | Instr::SlaPtr { .. }
            | Instr::SraPtr { .. }
            | Instr::SrlPtr { .. }
            | Instr::SwapPtr { .. }
            | Instr::Ccf { .. }
            | Instr::Scf { .. }
            | Instr::Split { .. }
            | Instr::MoveZero { .. }
            | Instr::MoveCarry { .. }
            | Instr::AddSP { .. }
            | Instr::PushR16 { .. }
            | Instr::PushAF { .. }
            | Instr::PopAF { .. } => None,
        }
    }

    #[must_use]
    pub fn zero(&self) -> Option<ZeroId> {
        match self {
            Instr::Adc { z, .. }
            | Instr::Add { z, .. }
            | Instr::Cp { z, .. }
            | Instr::Dec { z, .. }
            | Instr::DecPtr { z, .. }
            | Instr::Inc { z, .. }
            | Instr::IncPtr { z, .. }
            | Instr::Sbc { z, .. }
            | Instr::Sub { z, .. }
            | Instr::And { z, .. }
            | Instr::Or { z, .. }
            | Instr::Xor { z, .. }
            | Instr::Bit { z, .. }
            | Instr::BitPtr { z, .. }
            | Instr::Rl { z, .. }
            | Instr::RlPtr { z, .. }
            | Instr::Rlc { z, .. }
            | Instr::RlcPtr { z, .. }
            | Instr::Rr { z, .. }
            | Instr::RrPtr { z, .. }
            | Instr::Rrc { z, .. }
            | Instr::RrcPtr { z, .. }
            | Instr::Sla { z, .. }
            | Instr::SlaPtr { z, .. }
            | Instr::Sra { z, .. }
            | Instr::SraPtr { z, .. }
            | Instr::Srl { z, .. }
            | Instr::SrlPtr { z, .. }
            | Instr::Swap { z, .. }
            | Instr::SwapPtr { z, .. }
            | Instr::AddSP { z, .. }
            | Instr::PopAF { z, .. }
            | Instr::MoveZero { z, .. } => Some(*z),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Value { .. }
            | Instr::Ld8Ptr { .. }
            | Instr::Ld8HighPtr { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
            | Instr::StoreHigh { .. }
            | Instr::StoreValue { .. }
            | Instr::StoreInc { .. }
            | Instr::StoreDec { .. }
            | Instr::Cpl { .. }
            | Instr::Res { .. }
            | Instr::ResPtr { .. }
            | Instr::Set { .. }
            | Instr::SetPtr { .. }
            | Instr::Ccf { .. }
            | Instr::Scf { .. }
            | Instr::Add16 { .. }
            | Instr::LdSPE8 { .. }
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::PushR16 { .. }
            | Instr::PushAF { .. }
            | Instr::PopR16 { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::MoveCarry { .. } => None,
        }
    }

    #[must_use]
    pub fn carry(&self) -> Option<CarryId> {
        match self {
            Instr::Adc { c, .. }
            | Instr::Add { c, .. }
            | Instr::Cp { c, .. }
            | Instr::Sbc { c, .. }
            | Instr::Sub { c, .. }
            | Instr::And { c, .. }
            | Instr::Or { c, .. }
            | Instr::Xor { c, .. }
            | Instr::Rl { c, .. }
            | Instr::RlPtr { c, .. }
            | Instr::Rlc { c, .. }
            | Instr::RlcPtr { c, .. }
            | Instr::Rr { c, .. }
            | Instr::RrPtr { c, .. }
            | Instr::Rrc { c, .. }
            | Instr::RrcPtr { c, .. }
            | Instr::Sla { c, .. }
            | Instr::SlaPtr { c, .. }
            | Instr::Sra { c, .. }
            | Instr::SraPtr { c, .. }
            | Instr::Srl { c, .. }
            | Instr::SrlPtr { c, .. }
            | Instr::Swap { c, .. }
            | Instr::SwapPtr { c, .. }
            | Instr::Ccf { c, .. }
            | Instr::Scf { c, .. }
            | Instr::Add16 { c, .. }
            | Instr::AddSP { c, .. }
            | Instr::LdSPE8 { c, .. }
            | Instr::PopAF { c, .. }
            | Instr::MoveCarry { c, .. } => Some(*c),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Value { .. }
            | Instr::Ld8Ptr { .. }
            | Instr::Ld8HighPtr { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
            | Instr::StoreHigh { .. }
            | Instr::StoreValue { .. }
            | Instr::StoreInc { .. }
            | Instr::StoreDec { .. }
            | Instr::Dec { .. }
            | Instr::DecPtr { .. }
            | Instr::Inc { .. }
            | Instr::IncPtr { .. }
            | Instr::Cpl { .. }
            | Instr::Bit { .. }
            | Instr::BitPtr { .. }
            | Instr::Res { .. }
            | Instr::ResPtr { .. }
            | Instr::Set { .. }
            | Instr::SetPtr { .. }
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::PushR16 { .. }
            | Instr::PushAF { .. }
            | Instr::PopR16 { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::MoveZero { .. } => None,
        }
    }

    pub fn flags(&self) -> impl Iterator<Item = FlagId> + '_ {
        self.zero()
            .into_iter()
            .map(FlagId::from)
            .chain(self.carry().map(FlagId::from))
    }

    pub fn source_r8(&self) -> impl Iterator<Item = R8Id> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let r8 = match self {
                Instr::Adc { left, right, .. }
                | Instr::Add { left, right, .. }
                | Instr::Cp { left, right, .. }
                | Instr::Sbc { left, right, .. }
                | Instr::Sub { left, right, .. }
                | Instr::And { left, right, .. }
                | Instr::Or { left, right, .. }
                | Instr::Xor { left, right, .. } => match next {
                    0 => Some(left.0),
                    1 => match right {
                        Operand8::Reg(right) => Some(*right),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Join {
                    high_in, low_in, ..
                } => match next {
                    0 => Some(*low_in),
                    1 => Some(*high_in),
                    _ => None,
                },
                Instr::StoreHigh {
                    addr: target_addr,
                    source,
                    ..
                } => match next {
                    0 => Some(source.0),
                    1 => match target_addr {
                        HighAddr::Reg(addr) => Some(*addr),
                        HighAddr::Literal(_) => None,
                    },
                    _ => None,
                },
                Instr::Ld8 { source, .. }
                | Instr::Ld8HighPtr {
                    source_addr: HighAddr::Reg(source),
                    ..
                }
                | Instr::Store { source, .. }
                | Instr::Dec { source, .. }
                | Instr::Inc { source, .. }
                | Instr::Bit { source, .. }
                | Instr::Res { source, .. }
                | Instr::Set { source, .. }
                | Instr::Rl { source, .. }
                | Instr::Rlc { source, .. }
                | Instr::Rr { source, .. }
                | Instr::Rrc { source, .. }
                | Instr::Sla { source, .. }
                | Instr::Sra { source, .. }
                | Instr::Srl { source, .. }
                | Instr::Swap { source, .. } => match next {
                    0 => Some(*source),
                    _ => None,
                },
                Instr::StoreInc { source, .. }
                | Instr::StoreDec { source, .. }
                | Instr::Cpl { source, .. }
                | Instr::PushAF { source, .. } => match next {
                    0 => Some(source.0),
                    _ => None,
                },
                Instr::Nop
                | Instr::Ld8Value { .. }
                | Instr::Ld8Ptr { .. }
                | Instr::Ld8HighPtr { .. }
                | Instr::Ld8Inc { .. }
                | Instr::Ld8Dec { .. }
                | Instr::Ld16 { .. }
                | Instr::StoreValue { .. }
                | Instr::DecPtr { .. }
                | Instr::IncPtr { .. }
                | Instr::BitPtr { .. }
                | Instr::ResPtr { .. }
                | Instr::SetPtr { .. }
                | Instr::RlPtr { .. }
                | Instr::RlcPtr { .. }
                | Instr::RrPtr { .. }
                | Instr::RrcPtr { .. }
                | Instr::SlaPtr { .. }
                | Instr::SraPtr { .. }
                | Instr::SrlPtr { .. }
                | Instr::SwapPtr { .. }
                | Instr::Ccf { .. }
                | Instr::Scf { .. }
                | Instr::Add16 { .. }
                | Instr::Dec16 { .. }
                | Instr::Inc16 { .. }
                | Instr::MoveZero { .. }
                | Instr::MoveCarry { .. }
                | Instr::Split { .. }
                | Instr::AddSP { .. }
                | Instr::LdSPE8 { .. }
                | Instr::PushR16 { .. }
                | Instr::PopR16 { .. }
                | Instr::PopAF { .. } => None,
            }?;
            next += 1;
            Some(r8)
        })
    }

    pub fn source_r16(&self) -> impl Iterator<Item = R16Id> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let r16 = match self {
                Instr::Add16 { left, right, .. } => match next {
                    0 => Some(left.0),
                    1 => Some(*right),
                    _ => None,
                },
                Instr::Ld8Ptr {
                    source_addr: addr, ..
                }
                | Instr::Store {
                    target_addr: addr, ..
                } => match next {
                    0 => match addr {
                        Addr::Reg(addr) => Some(*addr),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Ld8Inc {
                    source_addr: addr, ..
                }
                | Instr::Ld8Dec {
                    source_addr: addr, ..
                }
                | Instr::StoreInc { addr, .. }
                | Instr::StoreDec { addr, .. }
                | Instr::DecPtr { addr, .. }
                | Instr::IncPtr { addr, .. }
                | Instr::BitPtr {
                    source_addr: addr, ..
                }
                | Instr::ResPtr { addr, .. }
                | Instr::SetPtr { addr, .. }
                | Instr::RlPtr { addr, .. }
                | Instr::RlcPtr { addr, .. }
                | Instr::RrPtr { addr, .. }
                | Instr::RrcPtr { addr, .. }
                | Instr::SlaPtr { addr, .. }
                | Instr::SraPtr { addr, .. }
                | Instr::SrlPtr { addr, .. }
                | Instr::SwapPtr { addr, .. } => match next {
                    0 => Some(addr.0),
                    _ => None,
                },
                Instr::Dec16 { source, .. }
                | Instr::Inc16 { source, .. }
                | Instr::Split { source, .. }
                | Instr::PushR16 { source, .. } => match next {
                    0 => Some(*source),
                    _ => None,
                },
                Instr::Adc { right, .. }
                | Instr::Add { right, .. }
                | Instr::Cp { right, .. }
                | Instr::Sbc { right, .. }
                | Instr::Sub { right, .. }
                | Instr::And { right, .. }
                | Instr::Or { right, .. }
                | Instr::Xor { right, .. } => match next {
                    0 => match right {
                        Operand8::Ptr(right) => Some(right.0),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Nop
                | Instr::Ld8 { .. }
                | Instr::Ld8Value { .. }
                | Instr::Ld8HighPtr { .. }
                | Instr::Ld16 { .. }
                | Instr::StoreValue { .. }
                | Instr::StoreHigh { .. }
                | Instr::Dec { .. }
                | Instr::Inc { .. }
                | Instr::Cpl { .. }
                | Instr::Bit { .. }
                | Instr::Res { .. }
                | Instr::Set { .. }
                | Instr::Rl { .. }
                | Instr::Rlc { .. }
                | Instr::Rr { .. }
                | Instr::Rrc { .. }
                | Instr::Sla { .. }
                | Instr::Sra { .. }
                | Instr::Srl { .. }
                | Instr::Swap { .. }
                | Instr::Ccf { .. }
                | Instr::Scf { .. }
                | Instr::Join { .. }
                | Instr::MoveZero { .. }
                | Instr::MoveCarry { .. }
                | Instr::AddSP { .. }
                | Instr::LdSPE8 { .. }
                | Instr::PushAF { .. }
                | Instr::PopR16 { .. }
                | Instr::PopAF { .. } => None,
            }?;
            next += 1;
            Some(r16)
        })
    }

    #[must_use]
    pub fn zero_in(&self) -> Option<ZeroId> {
        match self {
            Instr::PushAF { z_in, .. } | Instr::MoveZero { z_in, .. } => Some(*z_in),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Value { .. }
            | Instr::Ld8Ptr { .. }
            | Instr::Ld8HighPtr { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
            | Instr::StoreHigh { .. }
            | Instr::StoreValue { .. }
            | Instr::StoreInc { .. }
            | Instr::StoreDec { .. }
            | Instr::Adc { .. }
            | Instr::Add { .. }
            | Instr::Cp { .. }
            | Instr::Dec { .. }
            | Instr::DecPtr { .. }
            | Instr::Inc { .. }
            | Instr::IncPtr { .. }
            | Instr::Sbc { .. }
            | Instr::Sub { .. }
            | Instr::And { .. }
            | Instr::Cpl { .. }
            | Instr::Or { .. }
            | Instr::Xor { .. }
            | Instr::Bit { .. }
            | Instr::BitPtr { .. }
            | Instr::Res { .. }
            | Instr::ResPtr { .. }
            | Instr::Set { .. }
            | Instr::SetPtr { .. }
            | Instr::Rl { .. }
            | Instr::RlPtr { .. }
            | Instr::Rlc { .. }
            | Instr::RlcPtr { .. }
            | Instr::Rr { .. }
            | Instr::RrPtr { .. }
            | Instr::Rrc { .. }
            | Instr::RrcPtr { .. }
            | Instr::Sla { .. }
            | Instr::SlaPtr { .. }
            | Instr::Sra { .. }
            | Instr::SraPtr { .. }
            | Instr::Srl { .. }
            | Instr::SrlPtr { .. }
            | Instr::Swap { .. }
            | Instr::SwapPtr { .. }
            | Instr::Ccf { .. }
            | Instr::Scf { .. }
            | Instr::Add16 { .. }
            | Instr::AddSP { .. }
            | Instr::LdSPE8 { .. }
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::PushR16 { .. }
            | Instr::PopR16 { .. }
            | Instr::PopAF { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::MoveCarry { .. } => None,
        }
    }

    #[must_use]
    pub fn carry_in(&self) -> Option<CarryId> {
        match self {
            Instr::Adc { c_in, .. }
            | Instr::Sbc { c_in, .. }
            | Instr::Rl { c_in, .. }
            | Instr::RlPtr { c_in, .. }
            | Instr::Rr { c_in, .. }
            | Instr::RrPtr { c_in, .. }
            | Instr::Ccf { c_in, .. }
            | Instr::MoveCarry { c_in, .. }
            | Instr::PushAF { c_in, .. } => Some(*c_in),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Value { .. }
            | Instr::Ld8Ptr { .. }
            | Instr::Ld8HighPtr { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
            | Instr::StoreHigh { .. }
            | Instr::StoreValue { .. }
            | Instr::StoreInc { .. }
            | Instr::StoreDec { .. }
            | Instr::Add { .. }
            | Instr::Cp { .. }
            | Instr::Dec { .. }
            | Instr::DecPtr { .. }
            | Instr::Inc { .. }
            | Instr::IncPtr { .. }
            | Instr::Sub { .. }
            | Instr::And { .. }
            | Instr::Cpl { .. }
            | Instr::Or { .. }
            | Instr::Xor { .. }
            | Instr::Bit { .. }
            | Instr::BitPtr { .. }
            | Instr::Res { .. }
            | Instr::ResPtr { .. }
            | Instr::Set { .. }
            | Instr::SetPtr { .. }
            | Instr::Rlc { .. }
            | Instr::RlcPtr { .. }
            | Instr::Rrc { .. }
            | Instr::RrcPtr { .. }
            | Instr::Sla { .. }
            | Instr::SlaPtr { .. }
            | Instr::Sra { .. }
            | Instr::SraPtr { .. }
            | Instr::Srl { .. }
            | Instr::SrlPtr { .. }
            | Instr::Swap { .. }
            | Instr::SwapPtr { .. }
            | Instr::Scf { .. }
            | Instr::Add16 { .. }
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::AddSP { .. }
            | Instr::LdSPE8 { .. }
            | Instr::PopAF { .. }
            | Instr::PushR16 { .. }
            | Instr::PopR16 { .. }
            | Instr::MoveZero { .. } => None,
        }
    }
}

impl Instr {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Instr::Nop => write!(f, "NOP"),
            Instr::Ld8 { target, source } => write!(
                f,
                "LD {}, {}",
                target.display(lir, section),
                source.display(lir, section)
            ),
            Instr::Ld8Value { target, source } => {
                write!(f, "LD {}, ${source:02x}", target.display(lir, section))
            }
            Instr::Ld8Ptr {
                target,
                source_addr,
            } => write!(
                f,
                "LD {}, {}",
                target.display(lir, section),
                source_addr.display(lir, section)
            ),
            Instr::Ld8HighPtr {
                target,
                source_addr,
            } => write!(
                f,
                "LD {}, {}",
                target.display(lir, section),
                source_addr.display(lir, section)
            ),
            Instr::Ld8Inc {
                target,
                revised_addr: revised,
                source_addr: source,
            } => {
                write!(
                    f,
                    "LD {}, [{}++]",
                    target.display(lir, section),
                    HLId::display_revised_hl(*revised, *source, lir, section)
                )
            }
            Instr::Ld8Dec {
                target,
                revised_addr: revised,
                source_addr: source,
            } => {
                write!(
                    f,
                    "LD {}, [{}--]",
                    target.display(lir, section),
                    HLId::display_revised_hl(*revised, *source, lir, section)
                )
            }
            Instr::Ld16 { target, source } => {
                write!(
                    f,
                    "LD {}, {}",
                    target.display(lir, section),
                    source.display(lir, section)
                )
            }
            Instr::Store {
                target_addr,
                source,
            } => write!(
                f,
                "LD {}, {}",
                target_addr.display(lir, section),
                source.display(lir, section)
            ),
            Instr::StoreHigh {
                addr: target_addr,
                source,
            } => write!(
                f,
                "LD {}, {}",
                target_addr.display(lir, section),
                source.display(lir, section)
            ),
            Instr::StoreValue {
                addr: target_addr,
                source,
            } => write!(
                f,
                "LD [{}], ${source:02x}",
                target_addr.display(lir, section)
            ),
            Instr::StoreInc {
                revised_addr,
                addr: target_addr,
                source,
            } => write!(
                f,
                "LD [{}++], {}",
                HLId::display_revised_hl(*revised_addr, *target_addr, lir, section),
                source.display(lir, section)
            ),
            Instr::StoreDec {
                revised_addr,
                addr: target_addr,
                source,
            } => write!(
                f,
                "LD [{}--], {}",
                HLId::display_revised_hl(*revised_addr, *target_addr, lir, section),
                source.display(lir, section)
            ),
            Instr::Adc {
                target,
                z,
                c,
                left,
                right,
                c_in,
            } => {
                write!(
                    f,
                    "ADC {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::Add {
                target,
                z,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "ADD {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Cp { z, c, left, right } => {
                write!(
                    f,
                    "CP {}, {}; {}, {}",
                    left.display(lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Dec { target, z, source } => {
                write!(
                    f,
                    "DEC {}; {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::DecPtr { z, addr } => {
                write!(
                    f,
                    "DEC [{}]; {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::Inc { target, z, source } => {
                write!(
                    f,
                    "INC {}; {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::IncPtr { z, addr } => {
                write!(
                    f,
                    "INC [{}]; {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::Sbc {
                target,
                z,
                c,
                left,
                right,
                c_in,
            } => {
                write!(
                    f,
                    "SBC {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::Sub {
                target,
                z,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "SUB {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::And {
                target,
                z,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "AND {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Cpl { target, source } => {
                write!(
                    f,
                    "CPL {}",
                    AId::display_revised_a(*target, *source, lir, section)
                )
            }
            Instr::Or {
                target,
                z,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "OR {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Xor {
                target,
                z,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "XOR {}, {}; {}, {}",
                    AId::display_revised_a(*target, *left, lir, section),
                    right.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Bit { z, bit, source } => {
                write!(
                    f,
                    "BIT {bit}, {}; {}",
                    source.display(lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::BitPtr {
                z,
                bit,
                source_addr,
            } => {
                write!(
                    f,
                    "BIT {bit}, [{}]; {}",
                    source_addr.display(lir, section),
                    z.display_zero(lir, section)
                )
            }
            Instr::Res {
                target,
                bit,
                source,
            } => {
                write!(
                    f,
                    "RES {bit}, {}",
                    R8Id::display_revised(*target, *source, lir, section)
                )
            }
            Instr::ResPtr { bit, addr } => {
                write!(f, "RES {bit}, [{}]", addr.display(lir, section))
            }
            Instr::Set {
                target,
                bit,
                source,
            } => {
                write!(
                    f,
                    "SET {bit}, {}",
                    R8Id::display_revised(*target, *source, lir, section)
                )
            }
            Instr::SetPtr { bit, addr } => {
                write!(f, "SET {bit}, [{}]", addr.display(lir, section))
            }
            Instr::Rl {
                target,
                z,
                c,
                source,
                c_in,
            } => {
                write!(
                    f,
                    "RL {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::RlPtr { z, c, addr, c_in } => {
                write!(
                    f,
                    "RL [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::Rlc {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "RLC {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::RlcPtr { z, c, addr } => {
                write!(
                    f,
                    "RLC [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Rr {
                target,
                z,
                c,
                source,
                c_in,
            } => {
                write!(
                    f,
                    "RR {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::RrPtr { z, c, addr, c_in } => {
                write!(
                    f,
                    "RR [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::Rrc {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "RRC {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::RrcPtr { z, c, addr } => {
                write!(
                    f,
                    "RRC [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Sla {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "SLA {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::SlaPtr { z, c, addr } => {
                write!(
                    f,
                    "SLA [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Sra {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "SRA {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::SraPtr { z, c, addr } => {
                write!(
                    f,
                    "SRA [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Srl {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "SRL {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::SrlPtr { z, c, addr } => {
                write!(
                    f,
                    "SRL [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Swap {
                target,
                z,
                c,
                source,
            } => {
                write!(
                    f,
                    "SWAP {}; {}, {}",
                    R8Id::display_revised(*target, *source, lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::SwapPtr { z, c, addr } => {
                write!(
                    f,
                    "SWAP [{}]; {}, {}",
                    addr.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Ccf { c, c_in } => {
                write!(
                    f,
                    "CCF ; {})",
                    CarryId::display_revised_carry(*c, *c_in, lir, section)
                )
            }
            Instr::Scf { c } => {
                write!(f, "SCF ; {}", c.display_carry(lir, section))
            }
            Instr::Add16 {
                target,
                c,
                left,
                right,
            } => {
                write!(
                    f,
                    "ADD {}, {}; {}",
                    HLId::display_revised_hl(*target, *left, lir, section),
                    right.display(lir, section),
                    c.display_carry(lir, section)
                )
            }
            Instr::Dec16 { target, source } => {
                write!(
                    f,
                    "DEC {}",
                    R16Id::display_revised(*target, *source, lir, section)
                )
            }
            Instr::Inc16 { target, source } => {
                write!(
                    f,
                    "INC {}",
                    R16Id::display_revised(*target, *source, lir, section)
                )
            }
            Instr::AddSP { z, c, right } => {
                write!(
                    f,
                    "ADD SP {right}; {}, {}",
                    z.display_zero(lir, section),
                    c.display_carry(lir, section),
                )
            }
            Instr::LdSPE8 { target, c, right } => {
                write!(
                    f,
                    "LD {} SP+{right}; {}",
                    target.display(lir, section),
                    c.display_carry(lir, section),
                )
            }
            Instr::PushAF { source, z_in, c_in } => {
                write!(
                    f,
                    "PUSH AF(high_in={}, z_in={}, c_in={})",
                    source.display(lir, section),
                    z_in.display(lir, section),
                    c_in.display(lir, section),
                )
            }
            Instr::PushR16 { source } => {
                write!(f, "PUSH {}", source.display(lir, section))
            }
            Instr::PopAF { target, z, c } => {
                write!(
                    f,
                    "POP AF(high_out={}); {}, {}",
                    target.display(lir, section),
                    z.display_zero(lir, section),
                    c.display_carry(lir, section),
                )
            }
            Instr::PopR16 { target } => {
                write!(f, "POP {}", target.display(lir, section))
            }
            Instr::Join {
                target,
                high_in,
                low_in,
            } => {
                write!(
                    f,
                    "(out={};high_in={};low_in={})",
                    target.display(lir, section),
                    high_in.display(lir, section),
                    low_in.display(lir, section)
                )
            }
            Instr::Split { high, low, source } => {
                write!(
                    f,
                    "(high_out={};low_out={};in={})",
                    high.display(lir, section),
                    low.display(lir, section),
                    source.display(lir, section)
                )
            }
            Instr::MoveZero { z, z_in } => {
                write!(
                    f,
                    "(out={},in={})",
                    z.display(lir, section),
                    z_in.display(lir, section)
                )
            }
            Instr::MoveCarry { c, c_in } => {
                write!(
                    f,
                    "(out={},in={})",
                    c.display(lir, section),
                    c_in.display(lir, section)
                )
            }
        })
    }
}

#[derive(Debug)]
pub enum Terminator {
    /// `CALL n16`
    Call {
        callee_section: Option<SectionId>,
        callee_block: BlockId,
        return_block: BlockId,
    },
    /// `CALL cc,n16`
    CallIf {
        source: FlagId,
        negate: bool,
        callee_section: Option<SectionId>,
        callee_block: BlockId,
        return_block: BlockId,
        else_block: BlockId,
    },
    /// Call a local r16.
    ///
    /// This results in the following machine instructions
    /// ```text
    /// PUSH <return_block>
    /// JP <r16>
    /// ```
    /// Utilizes:
    /// `JP HL`
    CallR16 {
        callee: R16Id,
        return_block: BlockId,
    },
    /// `JP n16`
    Jp { next_block: BlockId },
    /// `JP cc,n16`
    JpIf {
        source: FlagId,
        negate: bool,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// `RET`
    Ret,
    /// `RET cc`
    RetIf {
        source: FlagId,
        negate: bool,
        else_block: BlockId,
    },
    /// `HALT`
    Halt { return_block: BlockId },
}

impl Terminator {
    pub fn register_uses(&self) -> impl Iterator<Item = VirtualReg> + '_ {
        match self {
            Terminator::CallIf { source, .. }
            | Terminator::RetIf { source, .. }
            | Terminator::JpIf { source, .. } => Some(match source {
                FlagId::Zero(flag) => VirtualReg::from(*flag),
                FlagId::Carry(flag) => VirtualReg::from(*flag),
            }),
            Terminator::Call { .. }
            | Terminator::CallR16 { .. }
            | Terminator::Jp { .. }
            | Terminator::Ret
            | Terminator::Halt { .. } => None,
        }
        .into_iter()
    }

    pub fn next_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let value = match self {
                Terminator::CallIf {
                    return_block: then_block,
                    else_block,
                    ..
                }
                | Terminator::JpIf {
                    then_block,
                    else_block,
                    ..
                } => match next {
                    0 => Some(*then_block),
                    1 => Some(*else_block),
                    _ => None,
                },
                Terminator::Call {
                    return_block: next_block,
                    ..
                }
                | Terminator::CallR16 {
                    return_block: next_block,
                    ..
                }
                | Terminator::Jp { next_block, .. }
                | Terminator::RetIf {
                    else_block: next_block,
                    ..
                }
                | Terminator::Halt {
                    return_block: next_block,
                    ..
                } => match next {
                    0 => Some(*next_block),
                    _ => None,
                },
                Terminator::Ret => None,
            }?;
            next += 1;
            Some(value)
        })
    }

    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Terminator::Call {
                callee_section,
                callee_block,
                return_block,
            } => {
                let section = callee_section.as_ref().copied().unwrap_or(section);
                let name = match &lir.sections[section].data {
                    SectionData::Code(data) => &data.functions[*callee_block],
                    _ => unimplemented!("expected code section"),
                };
                write!(f, "CALL {name}; return={return_block}")
            }
            Terminator::CallIf {
                source,
                negate,
                callee_section,
                callee_block,
                return_block,
                else_block,
            } => {
                write!(f, "CALL ")?;
                if *negate {
                    write!(f, "n")?;
                }
                write!(f, "{}", source.display(lir, section))?;
                let section = callee_section.as_ref().copied().unwrap_or(section);
                let name = match &lir.sections[section].data {
                    SectionData::Code(data) => &data.functions[*callee_block],
                    _ => unimplemented!("expected code section"),
                };
                write!(f, ", {name}; return={return_block}, else={else_block}")
            }
            Terminator::CallR16 {
                callee,
                return_block,
            } => {
                write!(
                    f,
                    "CALL {}; return={return_block}",
                    callee.display(lir, section)
                )
            }
            Terminator::Jp { next_block: block } => {
                write!(f, "JP {block}")
            }
            Terminator::JpIf {
                source,
                negate,
                then_block,
                else_block,
            } => {
                write!(f, "JP ")?;
                if *negate {
                    write!(f, "n")?;
                }
                write!(f, "{}", source.display(lir, section))?;
                write!(f, ", {then_block}; else={else_block}")
            }
            Terminator::Ret => {
                write!(f, "RET")
            }
            Terminator::RetIf {
                source,
                negate,
                else_block,
            } => {
                write!(f, "RET ")?;
                if *negate {
                    write!(f, "n")?;
                }
                write!(f, "{}", source.display(lir, section))?;
                write!(f, "; else={else_block}")
            }
            Terminator::Halt { return_block } => {
                write!(f, "HALT; return={return_block}")
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Addr {
    Reg(R16Id),
    Literal(u16),
    Section { section: SectionId, offset: u16 },
}

impl From<R16Id> for Addr {
    fn from(value: R16Id) -> Self {
        Addr::Reg(value)
    }
}

impl From<HLId> for Addr {
    fn from(value: HLId) -> Self {
        Addr::Reg(value.0)
    }
}

impl Addr {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Addr::Reg(addr) => write!(f, "[{}]", addr.display(lir, section)),
            Addr::Literal(addr) => write!(f, "[${addr:04x}]"),
            Addr::Section {
                section: addr_section,
                offset,
            } => {
                if *offset == 0 {
                    write!(f, "[{}]", addr_section.display(lir))
                } else {
                    write!(f, "[{}+${offset:04x}]", addr_section.display(lir))
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HighAddr {
    Reg(R8Id),
    Literal(u8),
}

impl HighAddr {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            HighAddr::Reg(addr) => write!(f, "[$ff00 + {}]", addr.display(lir, section)),
            HighAddr::Literal(addr) => write!(f, "[$ff00 + {addr}]"),
        })
    }
}

#[derive(Debug, Clone)]
pub enum Ld16Source {
    Literal(u16),
    FnAddr {
        section: Option<SectionId>,
        block: BlockId,
    },
    SectionAddr {
        section: SectionId,
        offset: u16,
    },
}

impl Ld16Source {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Ld16Source::Literal(value) => write!(f, "${value:04x}"),
            Ld16Source::FnAddr {
                section: section_id,
                block,
            } => {
                let section = section_id.as_ref().copied().unwrap_or(section);
                let name = match &lir.sections[section].data {
                    SectionData::Code(data) => &data.functions[*block],
                    _ => unimplemented!("expected code section"),
                };
                write!(f, "{name}")
            }
            Ld16Source::SectionAddr { section, offset } => {
                write!(f, "{}+${offset:04x}", section.display(lir))
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Operand8 {
    Ptr(HLId),
    Reg(R8Id),
    Literal(u8),
}

impl Operand8 {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Operand8::Ptr(addr) => write!(f, "[{}]", addr.display(lir, section)),
            Operand8::Reg(reg) => write!(f, "{}", reg.display(lir, section)),
            Operand8::Literal(value) => write!(f, "${value:02x}"),
        })
    }
}

#[derive(Debug, Default)]
pub struct LirBuilder {
    lir: Lir,
    sealed_code_sections: SecondaryMap<SectionId, CodeSectionBuilder>,
}

impl LirBuilder {
    #[must_use]
    pub fn build(mut self) -> Lir {
        for (section_id, builder) in self.sealed_code_sections {
            self.lir.sections[section_id] = builder.build();
        }
        self.lir
    }
}

impl LirBuilder {
    #[must_use]
    pub fn insert_ram_section(&mut self, data_section_data: RamSectionData) -> SectionId {
        self.lir.sections.insert(Section {
            data: SectionData::Ram(data_section_data),
        })
    }

    #[must_use]
    pub fn insert_data_section(&mut self, data_section_data: DataSectionData) -> SectionId {
        self.lir.sections.insert(Section {
            data: SectionData::Data(data_section_data),
        })
    }

    #[must_use]
    pub fn reserve_code_section(&mut self) -> SectionId {
        self.lir.sections.insert(Section {
            data: SectionData::Code(CodeSectionData::default()),
        })
    }

    pub fn seal_code_section(&mut self, section: SectionId, builder: CodeSectionBuilder) {
        assert!(
            !self.sealed_code_sections.contains_key(section),
            "{section:?} already sealed"
        );
        assert_matches!(
            self.lir.sections[section].data,
            SectionData::Code(_),
            "{section:?} not a code section"
        );
        self.sealed_code_sections.insert(section, builder);
    }
}

#[derive(Debug, Default)]
pub struct CodeSectionBuilder {
    data: CodeSectionData,
    terminated_blocks: SlotKeySet<BlockId>,
    outstanding_phis: SparseSecondaryMap<R8Id, SparseSlotKeySet<BlockId>>,
}

impl CodeSectionBuilder {
    #[must_use]
    pub fn build(self) -> Section {
        assert!(
            self.outstanding_phis.is_empty(),
            "uncompleted phi registers"
        );
        Section {
            data: SectionData::Code(self.data),
        }
    }
}

impl CodeSectionBuilder {
    #[must_use]
    pub fn new_function(&mut self, name: Identifier) -> BlockId {
        let start_block = self.data.blocks.insert(Block {
            instructions: Vec::new(),
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        self.data.functions.insert(start_block, name);
        start_block
    }

    #[must_use]
    pub fn new_block(&mut self) -> BlockId {
        self.data.blocks.insert(Block {
            instructions: Vec::new(),
            terminator: Terminator::Ret,
            previous: Vec::new(),
        })
    }

    pub fn set_terminator(&mut self, block: BlockId, terminator: Terminator) {
        debug_assert!(
            !self.terminated_blocks.contains(block),
            "block {block:?} is already terminated"
        );
        match &terminator {
            Terminator::Ret => {}
            Terminator::Call { return_block, .. }
            | Terminator::CallR16 { return_block, .. }
            | Terminator::Jp {
                next_block: return_block,
                ..
            }
            | Terminator::RetIf {
                else_block: return_block,
                ..
            }
            | Terminator::Halt { return_block, .. } => {
                self.data.blocks[*return_block].previous.push(block);
            }
            Terminator::CallIf {
                return_block: then_block,
                else_block,
                ..
            }
            | Terminator::JpIf {
                then_block,
                else_block,
                ..
            } => {
                self.data.blocks[*then_block].previous.push(block);
                self.data.blocks[*else_block].previous.push(block);
            }
        }
        self.data.blocks[block].terminator = terminator;
        self.terminated_blocks.insert(block);
    }

    fn new_r8(&mut self) -> R8Id {
        self.data.r8_registers.insert(Register::default())
    }

    fn new_a(&mut self) -> AId {
        AId(self.data.r8_registers.insert(Register {
            color: Some(Color::A),
            ..Default::default()
        }))
    }

    fn new_r8_precolored(&mut self, color: Color) -> R8Id {
        self.data.r8_registers.insert(Register {
            color: Some(color),
            ..Default::default()
        })
    }

    fn new_r16(&mut self) -> R16Id {
        self.data.r16_registers.insert(Register::default())
    }

    fn new_hl(&mut self) -> HLId {
        HLId(self.data.r16_registers.insert(Register {
            color: Some(Color::HL),
            ..Default::default()
        }))
    }

    fn new_bc(&mut self) -> R16Id {
        self.data.r16_registers.insert(Register {
            color: Some(Color::BC),
            ..Default::default()
        })
    }

    fn new_uncolored_zero_flag(&mut self) -> ZeroId {
        self.data.zero_registers.insert(Register::default())
    }

    fn new_uncolored_carry_flag(&mut self) -> CarryId {
        self.data.carry_registers.insert(Register::default())
    }

    fn new_zero_flag(&mut self) -> ZeroId {
        self.data.zero_registers.insert(Register {
            color: Some(Color::Fz),
            ..Default::default()
        })
    }

    fn new_carry_flag(&mut self) -> CarryId {
        self.data.carry_registers.insert(Register {
            color: Some(Color::Fc),
            ..Default::default()
        })
    }

    fn insert_instr(&mut self, block: BlockId, instr: Instr) {
        let index = self.data.blocks[block].instructions.len();
        for target in instr.target_r8s() {
            assert!(!self.data.r8_registers[target].defined.contains_key(block));
            self.data.r8_registers[target].defined.insert(block, index);
        }
        if let Some(target) = instr.target_r16() {
            assert!(!self.data.r16_registers[target].defined.contains_key(block));
            self.data.r16_registers[target].defined.insert(block, index);
        }
        if let Some(flag) = instr.zero() {
            assert!(!self.data.zero_registers[flag].defined.contains_key(block));
            self.data.zero_registers[flag].defined.insert(block, index);
        }
        if let Some(flag) = instr.carry() {
            assert!(!self.data.carry_registers[flag].defined.contains_key(block));
            self.data.carry_registers[flag].defined.insert(block, index);
        }
        self.data.blocks[block].instructions.push(instr);
    }

    #[must_use]
    pub fn reserve_phi(&mut self) -> R8Id {
        let r8 = self.new_r8();
        self.outstanding_phis.insert(r8, SparseSlotKeySet::new());
        r8
    }

    pub fn insert_phi_ld(&mut self, block: BlockId, target: R8Id, source: R8Id) {
        match self.outstanding_phis.entry(target).unwrap() {
            Entry::Occupied(mut entry) => assert!(
                entry.get_mut().insert(block),
                "block {block:?} already inserted load for phi register {target:?}"
            ),
            Entry::Vacant(_) => panic!("{target:?} not a phi register"),
        }
        self.insert_instr(block, Instr::Ld8 { target, source });
    }

    pub fn complete_phi(&mut self, target: R8Id, blocks: &[BlockId]) {
        let actual = self
            .outstanding_phis
            .remove(target)
            .unwrap_or_else(|| panic!("{target:?} already completed"));
        let expected = blocks.iter().copied().collect::<SparseSlotKeySet<_>>();
        assert_eq!(
            actual, expected,
            "missing phi loads for phi register {target:?}"
        );
    }

    #[must_use]
    pub fn insert_ld8_operand(&mut self, block: BlockId, source: Operand8) -> R8Id {
        match source {
            Operand8::Ptr(source_addr) => self.insert_ld8_hl_ptr(block, source_addr),
            Operand8::Reg(source) => self.insert_ld8(block, source),
            Operand8::Literal(source) => self.insert_ld8_value(block, source),
        }
    }

    /// `LD r8,r8`
    #[must_use]
    pub fn insert_ld8(&mut self, block: BlockId, source: R8Id) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::Ld8 { target, source });
        target
    }

    /// `LD A,r8`
    #[must_use]
    pub fn insert_ld_a(&mut self, block: BlockId, source: R8Id) -> AId {
        let target = self.new_a();
        self.insert_instr(
            block,
            Instr::Ld8 {
                target: target.0,
                source,
            },
        );
        target
    }

    /// `LD r8,A`
    #[must_use]
    pub fn insert_move_a(&mut self, block: BlockId, source: AId) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(
            block,
            Instr::Ld8 {
                target,
                source: source.0,
            },
        );
        target
    }

    /// `LD r8,n8`
    #[must_use]
    pub fn insert_ld8_value(&mut self, block: BlockId, source: u8) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::Ld8Value { target, source });
        target
    }

    /// `LD A,[r16]`
    /// `LD A,[n16]`
    #[must_use]
    pub fn insert_ld8_ptr(&mut self, block: BlockId, source_addr: Addr) -> AId {
        let target = self.new_a();
        self.insert_instr(
            block,
            Instr::Ld8Ptr {
                target: target.0,
                source_addr,
            },
        );
        target
    }

    /// `LD r8,[HL]`
    #[must_use]
    pub fn insert_ld8_hl_ptr(&mut self, block: BlockId, source_addr: HLId) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(
            block,
            Instr::Ld8Ptr {
                target,
                source_addr: Addr::Reg(source_addr.0),
            },
        );
        target
    }

    /// `LDH A,[n16]`
    /// `LDH A,[C]`
    #[must_use]
    pub fn insert_ld8_high_ptr(&mut self, block: BlockId, source_addr: HighAddr) -> AId {
        let target = self.new_a();
        self.insert_instr(
            block,
            Instr::Ld8HighPtr {
                target,
                source_addr,
            },
        );
        target
    }

    /// `LD A,[HLI]`
    #[must_use]
    pub fn insert_ld8_inc(&mut self, block: BlockId, source: HLId) -> (AId, HLId) {
        let target = self.new_a();
        let revised = self.new_hl();
        self.insert_instr(
            block,
            Instr::Ld8Inc {
                target,
                revised_addr: revised,
                source_addr: source,
            },
        );
        (target, revised)
    }

    /// `LD A,[HLD]`
    #[must_use]
    pub fn insert_ld8_dec(&mut self, block: BlockId, source: HLId) -> (AId, HLId) {
        let target = self.new_a();
        let revised = self.new_hl();
        self.insert_instr(
            block,
            Instr::Ld8Dec {
                target,
                revised_addr: revised,
                source_addr: source,
            },
        );
        (target, revised)
    }

    /// `LD r16,n16`
    #[must_use]
    pub fn insert_ld16(&mut self, block: BlockId, source: Ld16Source) -> R16Id {
        let target = self.new_r16();
        self.insert_instr(block, Instr::Ld16 { target, source });
        target
    }

    /// `LD HL,n16`
    #[must_use]
    pub fn insert_ld_hl(&mut self, block: BlockId, source: Ld16Source) -> HLId {
        let target = self.new_hl();
        self.insert_instr(
            block,
            Instr::Ld16 {
                target: target.0,
                source,
            },
        );
        target
    }

    /// `LD [HL],r8`
    pub fn insert_store_hl(&mut self, block: BlockId, target_addr: HLId, source: R8Id) {
        self.insert_instr(
            block,
            Instr::Store {
                target_addr: Addr::Reg(target_addr.0),
                source,
            },
        );
    }

    /// `LD [r16],A`
    /// `LD [n16],A`
    pub fn insert_store(&mut self, block: BlockId, target_addr: Addr, source: AId) {
        self.insert_instr(
            block,
            Instr::Store {
                target_addr,
                source: source.0,
            },
        );
    }

    /// `LDH [n16],A`
    /// `LDH [C],A`
    pub fn insert_store_high(&mut self, block: BlockId, target_addr: HighAddr, source: AId) {
        self.insert_instr(
            block,
            Instr::StoreHigh {
                addr: target_addr,
                source,
            },
        );
    }

    /// `LD [HL],n8`
    pub fn insert_store_value(&mut self, block: BlockId, addr: HLId, source: u8) {
        self.insert_instr(block, Instr::StoreValue { addr, source });
    }

    /// `LD [HLI],A`
    #[must_use]
    pub fn insert_store_inc(&mut self, block: BlockId, target_addr: HLId, source: AId) -> HLId {
        let revised_addr = self.new_hl();
        self.insert_instr(
            block,
            Instr::StoreInc {
                revised_addr,
                addr: target_addr,
                source,
            },
        );
        revised_addr
    }

    /// `LD [HLD],A`
    #[must_use]
    pub fn insert_store_dec(&mut self, block: BlockId, target_addr: HLId, source: AId) -> HLId {
        let revised_addr = self.new_hl();
        self.insert_instr(
            block,
            Instr::StoreDec {
                revised_addr,
                addr: target_addr,
                source,
            },
        );
        revised_addr
    }

    /// `ADC A,r8`
    /// `ADC A,[HL]`
    /// `ADC A,n8`
    #[must_use]
    pub fn insert_adc(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
        c_in: CarryId,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Adc {
                target,
                z,
                c,
                left,
                right,
                c_in,
            },
        );
        (target, z, c)
    }

    /// `ADD A,r8`
    /// `ADD A,[HL]`
    /// `ADD A,n8`
    #[must_use]
    pub fn insert_add(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Add {
                target,
                z,
                c,
                left,
                right,
            },
        );
        (target, z, c)
    }

    /// `CP A,r8`
    /// `CP A,[HL]`
    /// `CP A,n8`
    #[must_use]
    pub fn insert_cp(&mut self, block: BlockId, left: AId, right: Operand8) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Cp { z, c, left, right });
        (z, c)
    }

    /// `DEC r8`
    #[must_use]
    pub fn insert_dec(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Dec { target, z, source });
        (target, z)
    }

    /// `DEC [HL]`
    #[must_use]
    pub fn insert_dec_ptr(&mut self, block: BlockId, addr: HLId) -> ZeroId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::DecPtr { z, addr });
        z
    }

    /// `INC r8`
    #[must_use]
    pub fn insert_inc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Inc { target, z, source });
        (target, z)
    }

    /// `INC [HL]`
    #[must_use]
    pub fn insert_inc_ptr(&mut self, block: BlockId, addr: HLId) -> ZeroId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::IncPtr { z, addr });
        z
    }

    /// `SBC A,r8`
    /// `SBC A,[HL]`
    /// `SBC A,n8`
    #[must_use]
    pub fn insert_sbc(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
        c_in: CarryId,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Sbc {
                target,
                z,
                c,
                left,
                right,
                c_in,
            },
        );
        (target, z, c)
    }

    /// `SUB A,r8`
    /// `SUB A,[HL]`
    /// `SUB A,n8`
    #[must_use]
    pub fn insert_sub(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Sub {
                target,
                z,
                c,
                left,
                right,
            },
        );
        (target, z, c)
    }

    /// `AND A,r8`
    /// `AND A,[HL]`
    /// `AND A,n8`
    #[must_use]
    pub fn insert_and(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::And {
                target,
                z,
                c,
                left,
                right,
            },
        );
        (target, z, c)
    }

    /// `CPL`
    #[must_use]
    pub fn insert_cpl(&mut self, block: BlockId, source: AId) -> AId {
        let target = self.new_a();
        self.insert_instr(block, Instr::Cpl { target, source });
        target
    }

    /// `OR A,r8`
    /// `OR A,[HL]`
    /// `OR A,n8`
    #[must_use]
    pub fn insert_or(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Or {
                target,
                z,
                c,
                left,
                right,
            },
        );
        (target, z, c)
    }

    /// `XOR A,r8`
    /// `XOR A,[HL]`
    /// `XOR A,n8`
    #[must_use]
    pub fn insert_xor(
        &mut self,
        block: BlockId,
        left: AId,
        right: Operand8,
    ) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Xor {
                target,
                z,
                c,
                left,
                right,
            },
        );
        (target, z, c)
    }

    /// `BIT u3,r8`
    #[must_use]
    pub fn insert_bit(&mut self, block: BlockId, bit: u3, source: R8Id) -> ZeroId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Bit { z, bit, source });
        z
    }

    /// `BIT u3,[HL]`
    #[must_use]
    pub fn insert_bit_ptr(&mut self, block: BlockId, bit: u3, addr: HLId) -> ZeroId {
        let z = self.new_zero_flag();
        self.insert_instr(
            block,
            Instr::BitPtr {
                z,
                bit,
                source_addr: addr,
            },
        );
        z
    }

    /// `RES u3,r8`
    #[must_use]
    pub fn insert_res(&mut self, block: BlockId, bit: u3, source: R8Id) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(
            block,
            Instr::Res {
                target,
                bit,
                source,
            },
        );
        target
    }

    /// `RES u3,[HL]`
    pub fn insert_res_ptr(&mut self, block: BlockId, bit: u3, addr: HLId) {
        self.insert_instr(block, Instr::ResPtr { bit, addr });
    }

    /// `SET u3,r8`
    #[must_use]
    pub fn insert_set(&mut self, block: BlockId, bit: u3, source: R8Id) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(
            block,
            Instr::Set {
                target,
                bit,
                source,
            },
        );
        target
    }

    /// `SET u3,[HL]`
    pub fn insert_set_ptr(&mut self, block: BlockId, bit: u3, addr: HLId) {
        self.insert_instr(block, Instr::SetPtr { bit, addr });
    }

    /// `RL r8`
    /// `RLA`
    #[must_use]
    pub fn insert_rl(
        &mut self,
        block: BlockId,
        source: R8Id,
        c_in: CarryId,
    ) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Rl {
                target,
                z,
                c,
                source,
                c_in,
            },
        );
        (target, z, c)
    }

    /// `RL [HL]`
    #[must_use]
    pub fn insert_rl_ptr(
        &mut self,
        block: BlockId,
        addr: HLId,
        c_in: CarryId,
    ) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RlPtr { z, c, addr, c_in });
        (z, c)
    }

    /// `RLC r8`
    /// `RLCA`
    #[must_use]
    pub fn insert_rlc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Rlc {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// `RLC [HL]`
    #[must_use]
    pub fn insert_rlc_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RlcPtr { z, c, addr });
        (z, c)
    }
    /// `RR r8`
    /// `RRA`
    #[must_use]
    pub fn insert_rr(
        &mut self,
        block: BlockId,
        source: R8Id,
        c_in: CarryId,
    ) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Rr {
                target,
                z,
                c,
                source,
                c_in,
            },
        );
        (target, z, c)
    }

    /// `RR [HL]`
    #[must_use]
    pub fn insert_rr_ptr(
        &mut self,
        block: BlockId,
        addr: HLId,
        c_in: CarryId,
    ) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RrPtr { z, c, addr, c_in });
        (z, c)
    }

    /// `RRC r8`
    /// `RRCA`
    #[must_use]
    pub fn insert_rrc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Rrc {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// `RRC [HL]`
    #[must_use]
    pub fn insert_rrc_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RrcPtr { z, c, addr });
        (z, c)
    }

    /// `SLA r8`
    #[must_use]
    pub fn insert_sla(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Sla {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// `SLA [HL]`
    #[must_use]
    pub fn insert_sla_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SlaPtr { z, c, addr });
        (z, c)
    }

    /// Shift right arithmetically (signed)
    /// `SRA r8`
    #[must_use]
    pub fn insert_sra(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Sra {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// Shift right arithmetically (signed)
    /// `SRA [HL]`
    #[must_use]
    pub fn insert_sra_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SraPtr { z, c, addr });
        (z, c)
    }

    /// Shift right logically (unsigned)
    /// `SRL r8`
    #[must_use]
    pub fn insert_srl(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Srl {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// Shift right logically (unsigned)
    /// `SRL [HL]`
    #[must_use]
    pub fn insert_srl_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SrlPtr { z, c, addr });
        (z, c)
    }

    /// `SWAP r8`
    #[must_use]
    pub fn insert_swap(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroId, CarryId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Swap {
                target,
                z,
                c,
                source,
            },
        );
        (target, z, c)
    }

    /// `SWAP [HL]`
    #[must_use]
    pub fn insert_swap_ptr(&mut self, block: BlockId, addr: HLId) -> (ZeroId, CarryId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SwapPtr { z, c, addr });
        (z, c)
    }

    /// CCF
    #[must_use]
    pub fn insert_ccf(&mut self, block: BlockId, c_in: CarryId) -> CarryId {
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Ccf { c, c_in });
        c
    }

    /// SCF
    #[must_use]
    pub fn insert_scf(&mut self, block: BlockId) -> CarryId {
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Scf { c });
        c
    }

    /// `ADD HL,r16`
    #[must_use]
    pub fn insert_add16(&mut self, block: BlockId, left: HLId, right: R16Id) -> (HLId, CarryId) {
        let target = self.new_hl();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Add16 {
                target,
                c,
                left,
                right,
            },
        );
        (target, c)
    }

    /// `DEC r16`
    #[must_use]
    pub fn insert_dec16(&mut self, block: BlockId, source: R16Id) -> R16Id {
        let target = self.new_r16();
        self.insert_instr(block, Instr::Dec16 { target, source });
        target
    }

    /// `INC r16`
    #[must_use]
    pub fn insert_inc16(&mut self, block: BlockId, source: R16Id) -> R16Id {
        let target = self.new_r16();
        self.insert_instr(block, Instr::Inc16 { target, source });
        target
    }

    #[must_use]
    pub fn insert_split(&mut self, block: BlockId, source: R16Id) -> (R8Id, R8Id) {
        let (high, low) = match self.data.r16_registers[source].color {
            Some(Color::HL) => {
                let low = self.new_r8_precolored(Color::L);
                let high = self.new_r8_precolored(Color::H);
                (high, low)
            }
            Some(Color::BC) => {
                let low = self.new_r8_precolored(Color::C);
                let high = self.new_r8_precolored(Color::B);
                (high, low)
            }
            _ => {
                let low = self.new_r8();
                let high = self.new_r8();
                (high, low)
            }
        };
        self.insert_instr(block, Instr::Split { high, low, source });
        (high, low)
    }

    #[must_use]
    pub fn insert_join(&mut self, block: BlockId, high_in: R8Id, low_in: R8Id) -> R16Id {
        let target = match (
            self.data.r8_registers[high_in].color,
            self.data.r8_registers[low_in].color,
        ) {
            (Some(Color::H), Some(Color::L)) => self.new_hl().0,
            (Some(Color::B), Some(Color::C)) => self.new_bc(),
            _ => self.new_r16(),
        };
        self.insert_instr(
            block,
            Instr::Join {
                target,
                high_in,
                low_in,
            },
        );
        target
    }

    /// `SBC A, A`
    /// Useful for converting a carry flag into `0xFF` if carry set or `0x00` is not set
    #[must_use]
    pub fn insert_sbc_a_a(&mut self, block: BlockId, c_in: CarryId) -> (AId, ZeroId, CarryId) {
        let target = self.new_a();
        let left = self.new_a();
        let right = self.new_a();
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::Sbc {
                target,
                z,
                c,
                left,
                right: Operand8::Reg(right.0),
                c_in,
            },
        );
        (target, z, c)
    }

    /// `PUSH AF`
    /// Useful for storing the zero flag to the stack to be restored via a pop
    pub fn insert_push_af_z(&mut self, block: BlockId, z_in: ZeroId) {
        let source = self.new_a();
        let c_in = self.new_carry_flag();
        self.insert_instr(block, Instr::PushAF { source, z_in, c_in });
    }

    /// `POP r16`
    #[must_use]
    pub fn insert_pop_r16(&mut self, block: BlockId) -> R16Id {
        let target = self.new_r16();
        self.insert_instr(block, Instr::PopR16 { target });
        target
    }

    #[must_use]
    pub fn insert_move_zero(&mut self, block: BlockId, z_in: ZeroId) -> ZeroId {
        let z = self.new_uncolored_zero_flag();
        self.insert_instr(block, Instr::MoveZero { z, z_in });
        z
    }

    #[must_use]
    pub fn insert_move_carry(&mut self, block: BlockId, c_in: CarryId) -> CarryId {
        let c = self.new_uncolored_carry_flag();
        self.insert_instr(block, Instr::MoveCarry { c, c_in });
        c
    }

    #[must_use]
    pub fn insert_ld_return_8(&mut self, block: BlockId) -> R8Id {
        let source = self.new_a();
        let target = self.new_r8();
        self.insert_instr(
            block,
            Instr::Ld8 {
                target,
                source: source.0,
            },
        );
        target
    }

    #[must_use]
    pub fn insert_ld_return_16(&mut self, block: BlockId) -> (R8Id, R8Id) {
        let source_low = self.new_r8_precolored(Color::C);
        let source_high = self.new_r8_precolored(Color::B);
        let target_low = self.new_r8();
        let target_high = self.new_r8();
        self.insert_instr(
            block,
            Instr::Ld8 {
                target: target_low,
                source: source_low,
            },
        );
        self.insert_instr(
            block,
            Instr::Ld8 {
                target: target_high,
                source: source_high,
            },
        );
        (target_high, target_low)
    }
}
