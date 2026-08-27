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
use quokka_lang_slotkeyset::{SlotKeySet, SparseSlotKeySet};
use slotmap::sparse_secondary::Entry;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap, new_key_type};
use std::assert_matches;
use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroU16;

new_key_type! { pub struct SectionId; }
new_key_type! { pub struct BlockId; }
new_key_type! { pub struct ZeroFlagId; }
new_key_type! { pub struct CarryFlagId; }
new_key_type! { pub struct R8Id; }
new_key_type! { pub struct R16Id; }

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

impl fmt::Display for ZeroFlagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "z#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

impl fmt::Display for CarryFlagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

impl fmt::Display for R8Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r8#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

impl fmt::Display for R16Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r16#{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VirtualReg {
    R8(R8Id),
    R16(R16Id),
    Zero(ZeroFlagId),
    Carry(CarryFlagId),
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
    pub zero_flags: SlotMap<ZeroFlagId, Register>,
    pub carry_flags: SlotMap<CarryFlagId, Register>,
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
    previous: Vec<BlockId>,
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

#[derive(Debug, Clone)]
pub struct Register {
    pub defined: SparseSecondaryMap<BlockId, usize>,
}

#[derive(Debug)]
pub enum Instr {
    /// `NOP`
    Nop,
    /// `LD r8,r8`
    /// `LD r8,n8`
    /// `LD r8,[HL]`
    /// `LD A,[r16]`
    /// `LD A,[n16]`
    /// `LDH A,[n16]`
    /// `LDH A,[C]`
    Ld8 { target: R8Id, source: Ld8Source },
    /// `LD A,[HLI]`
    Ld8Inc {
        target: R8Id,
        revised: R16Id,
        source: R16Id,
    },
    /// `LD A,[HLD]`
    Ld8Dec {
        target: R8Id,
        revised: R16Id,
        source: R16Id,
    },
    /// `LD r16,n16`
    Ld16 { target: R16Id, source: Ld16Source },
    /// `LD [HL],r8`
    /// `LD [r16],A`
    /// `LD [n16],A`
    /// `LDH [n16],A`
    /// `LDH [C],A`
    Store { target: StoreTarget, source: R8Id },
    /// `LD [HL],n8`
    StoreValue { target: R16Id, source: u8 },
    /// `LD [HLI],A`
    StoreInc {
        revised: R16Id,
        left: R16Id,
        right: R8Id,
    },
    /// `LD [HLD],A`
    StoreDec {
        revised: R16Id,
        left: R16Id,
        right: R8Id,
    },
    /// `ADC A,r8`
    /// `ADC A,[HL]`
    /// `ADC A,n8`
    Adc {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
        c_in: CarryFlagId,
    },
    /// `ADD A,r8`
    /// `ADD A,[HL]`
    /// `ADD A,n8`
    Add {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `CP A,r8`
    /// `CP A,[HL]`
    /// `CP A,n8`
    Cp {
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `DEC r8`
    Dec {
        target: R8Id,
        z: ZeroFlagId,
        source: R8Id,
    },
    /// `DEC [HL]`
    DecPtr { z: ZeroFlagId, addr: R16Id },
    /// `INC r8`
    Inc {
        target: R8Id,
        z: ZeroFlagId,
        source: R8Id,
    },
    /// `INC [HL]`
    IncPtr { z: ZeroFlagId, addr: R16Id },
    /// `SBC A,r8`
    /// `SBC A,[HL]`
    /// `SBC A,n8`
    Sbc {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
        c_in: CarryFlagId,
    },
    /// `SUB A,r8`
    /// `SUB A,[HL]`
    /// `SUB A,n8`
    Sub {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `AND A,r8`
    /// `AND A,[HL]`
    /// `AND A,n8`
    And {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `CPL`
    Cpl { target: R8Id, source: R8Id },
    /// `OR A,r8`
    /// `OR A,[HL]`
    /// `OR A,n8`
    Or {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `XOR A,r8`
    /// `XOR A,[HL]`
    /// `XOR A,n8`
    Xor {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        left: R8Id,
        right: Operand8,
    },
    /// `BIT u3,r8`
    Bit {
        z: ZeroFlagId,
        bit: u3,
        source: R8Id,
    },
    /// `BIT u3,[HL]`
    BitPtr { z: ZeroFlagId, bit: u3, addr: R16Id },
    /// `RES u3,r8`
    Res { target: R8Id, bit: u3, source: R8Id },
    /// `RES u3,[HL]`
    ResPtr { bit: u3, addr: R16Id },
    /// `SET u3,r8`
    Set { target: R8Id, bit: u3, source: R8Id },
    /// `SET u3,[HL]`
    SetPtr { bit: u3, addr: R16Id },
    /// `RL r8`
    /// `RLA`
    Rl {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
        c_in: CarryFlagId,
    },
    /// `RL [HL]`
    RlPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
        c_in: CarryFlagId,
    },
    /// `RLC r8`
    /// `RLCA`
    Rlc {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `RLC [HL]`
    RlcPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `RR r8`
    /// `RRA`
    Rr {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
        c_in: CarryFlagId,
    },
    /// `RR [HL]`
    RrPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
        c_in: CarryFlagId,
    },
    /// `RRC r8`
    /// `RRCA`
    Rrc {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `RRC [HL]`
    RrcPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `SLA r8`
    Sla {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `SLA [HL]`
    SlaPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `SRA r8`
    Sra {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `SRA [HL]`
    SraPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `SRL r8`
    Srl {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `SRL [HL]`
    SrlPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `SWAP r8`
    Swap {
        target: R8Id,
        z: ZeroFlagId,
        c: CarryFlagId,
        source: R8Id,
    },
    /// `SWAP [HL]`
    SwapPtr {
        z: ZeroFlagId,
        c: CarryFlagId,
        addr: R16Id,
    },
    /// `CCF`
    Ccf { c: CarryFlagId, c_in: CarryFlagId },
    /// `SCF`
    Scf { c: CarryFlagId },
    /// `ADD HL,r16`
    Add16 {
        target: R16Id,
        c: CarryFlagId,
        left: R16Id,
        right: R16Id,
    },
    /// `DEC r16`
    Dec16 { target: R16Id, source: R16Id },
    /// `INC r16`
    Inc16 { target: R16Id, source: R16Id },
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
    /// Virtual instruction to convert a zero flag into an 8-bit register.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    ///
    /// Zero flag is guaranteed set in bit [`ZERO_FLAG_BIT`].
    /// All other bits have undefined behavior.
    LdZero { target: R8Id, z_in: ZeroFlagId },
    /// Virtual instruction to convert a carry flag into an 8-bit register.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    ///
    /// Carry flag is guaranteed set in bit [`CARRY_FLAG_BIT`].
    /// All other bits have undefined behavior.
    LdCarry { target: R8Id, c_in: CarryFlagId },
    /// Virtual instruction to capture the return value of a function call with an 8-bit return
    /// type.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    ///
    /// Calling convention is to set the return value of the `A` register
    LdReturn8 { target: R8Id },
    /// Virtual instruction to capture the return value of a function call with an 16-bit return
    /// type.
    ///
    /// This does not add any machine instructions on its own, it simply is used
    /// to force constraints on the register allocation.
    ///
    /// Calling convention is to set the return value of the `HL` register
    LdReturn16 { target: R16Id },
}

impl Instr {
    pub fn target_r8(&self) -> impl Iterator<Item = R8Id> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let r8 = match self {
                Instr::Split { high, low, .. } => match next {
                    0 => Some(*low),
                    1 => Some(*high),
                    _ => None,
                },
                Instr::Ld8 { target, .. }
                | Instr::Ld8Inc { target, .. }
                | Instr::Ld8Dec { target, .. }
                | Instr::Adc { target, .. }
                | Instr::Add { target, .. }
                | Instr::Dec { target, .. }
                | Instr::Inc { target, .. }
                | Instr::Sbc { target, .. }
                | Instr::Sub { target, .. }
                | Instr::And { target, .. }
                | Instr::Cpl { target, .. }
                | Instr::Or { target, .. }
                | Instr::Xor { target, .. }
                | Instr::Res { target, .. }
                | Instr::Set { target, .. }
                | Instr::Rl { target, .. }
                | Instr::Rlc { target, .. }
                | Instr::Rr { target, .. }
                | Instr::Rrc { target, .. }
                | Instr::Sla { target, .. }
                | Instr::Sra { target, .. }
                | Instr::Srl { target, .. }
                | Instr::Swap { target, .. }
                | Instr::LdZero { target, .. }
                | Instr::LdCarry { target, .. }
                | Instr::LdReturn8 { target, .. } => match next {
                    0 => Some(*target),
                    _ => None,
                },
                Instr::Nop
                | Instr::Ld16 { .. }
                | Instr::Store { .. }
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
                | Instr::LdReturn16 { .. } => None,
            }?;
            next += 1;
            Some(r8)
        })
    }

    #[must_use]
    pub fn target_r16(&self) -> Option<R16Id> {
        match self {
            Instr::Add16 { target, .. }
            | Instr::Dec16 { target, .. }
            | Instr::Inc16 { target, .. }
            | Instr::Join { target, .. }
            | Instr::LdReturn16 { target, .. }
            | Instr::Ld8Inc {
                revised: target, ..
            }
            | Instr::Ld8Dec {
                revised: target, ..
            }
            | Instr::StoreInc {
                revised: target, ..
            }
            | Instr::StoreDec {
                revised: target, ..
            } => Some(*target),
            Instr::Nop
            | Instr::Ld8 { .. }
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
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
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
            | Instr::LdZero { .. }
            | Instr::LdCarry { .. }
            | Instr::LdReturn8 { .. } => None,
        }
    }

    #[must_use]
    pub fn zero_flag(&self) -> Option<ZeroFlagId> {
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
            | Instr::SwapPtr { z, .. } => Some(*z),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
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
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::LdZero { .. }
            | Instr::LdCarry { .. }
            | Instr::LdReturn8 { .. }
            | Instr::LdReturn16 { .. } => None,
        }
    }

    #[must_use]
    pub fn carry_flag(&self) -> Option<CarryFlagId> {
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
            | Instr::Add16 { c, .. } => Some(*c),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
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
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::LdZero { .. }
            | Instr::LdCarry { .. }
            | Instr::LdReturn8 { .. }
            | Instr::LdReturn16 { .. } => None,
        }
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
                    0 => Some(*left),
                    1 => match right {
                        Operand8::Direct(right) => Some(*right),
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
                Instr::Ld8 { source, .. } => match next {
                    0 => match source {
                        Ld8Source::Operand(Operand8::Direct(source))
                        | Ld8Source::HighPtr(source) => Some(*source),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Store { source, .. }
                | Instr::StoreInc { right: source, .. }
                | Instr::StoreDec { right: source, .. }
                | Instr::Dec { source, .. }
                | Instr::Inc { source, .. }
                | Instr::Cpl { source, .. }
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
                Instr::Nop
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
                | Instr::LdZero { .. }
                | Instr::LdCarry { .. }
                | Instr::LdReturn8 { .. }
                | Instr::LdReturn16 { .. }
                | Instr::Split { .. } => None,
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
                    0 => Some(*left),
                    1 => Some(*right),
                    _ => None,
                },
                Instr::Ld8 { source, .. } => match next {
                    0 => match source {
                        Ld8Source::Operand(Operand8::Ptr(source)) => Some(*source),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Ld8Inc { source, .. }
                | Instr::Ld8Dec { source, .. }
                | Instr::StoreInc { left: source, .. }
                | Instr::StoreDec { left: source, .. }
                | Instr::DecPtr { addr: source, .. }
                | Instr::IncPtr { addr: source, .. }
                | Instr::BitPtr { addr: source, .. }
                | Instr::ResPtr { addr: source, .. }
                | Instr::SetPtr { addr: source, .. }
                | Instr::RlPtr { addr: source, .. }
                | Instr::RlcPtr { addr: source, .. }
                | Instr::RrPtr { addr: source, .. }
                | Instr::RrcPtr { addr: source, .. }
                | Instr::SlaPtr { addr: source, .. }
                | Instr::SraPtr { addr: source, .. }
                | Instr::SrlPtr { addr: source, .. }
                | Instr::SwapPtr { addr: source, .. }
                | Instr::Dec16 { source, .. }
                | Instr::Inc16 { source, .. }
                | Instr::Split { source, .. } => match next {
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
                        Operand8::Ptr(right) => Some(*right),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Nop
                | Instr::Ld16 { .. }
                | Instr::Store { .. }
                | Instr::StoreValue { .. }
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
                | Instr::LdZero { .. }
                | Instr::LdCarry { .. }
                | Instr::LdReturn8 { .. }
                | Instr::LdReturn16 { .. } => None,
            }?;
            next += 1;
            Some(r16)
        })
    }

    #[must_use]
    pub fn zero_flag_in(&self) -> Option<ZeroFlagId> {
        match self {
            Instr::LdZero { z_in, .. } => Some(*z_in),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
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
            | Instr::Dec16 { .. }
            | Instr::Inc16 { .. }
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::LdCarry { .. }
            | Instr::LdReturn8 { .. }
            | Instr::LdReturn16 { .. } => None,
        }
    }

    #[must_use]
    pub fn carry_flag_in(&self) -> Option<CarryFlagId> {
        match self {
            Instr::Adc { c_in, .. }
            | Instr::Sbc { c_in, .. }
            | Instr::Rl { c_in, .. }
            | Instr::RlPtr { c_in, .. }
            | Instr::Rr { c_in, .. }
            | Instr::RrPtr { c_in, .. }
            | Instr::Ccf { c_in, .. }
            | Instr::LdCarry { c_in, .. } => Some(*c_in),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
            | Instr::Ld16 { .. }
            | Instr::Store { .. }
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
            | Instr::LdZero { .. }
            | Instr::LdReturn8 { .. }
            | Instr::LdReturn16 { .. } => None,
        }
    }
}

impl Instr {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir, section: SectionId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Instr::Nop => write!(f, "NOP"),
            Instr::Ld8 { target, source } => write!(f, "LD {target}, {}", source.display(lir)),
            Instr::Ld8Inc {
                target,
                revised,
                source,
            } => {
                write!(f, "LD {target}, [I(out={revised};in={source})++]")
            }
            Instr::Ld8Dec {
                target,
                revised,
                source,
            } => {
                write!(f, "LD {target}, [(out={revised};in={source})--]")
            }
            Instr::Ld16 { target, source } => {
                write!(f, "LD {target}, {}", source.display(lir, section))
            }
            Instr::Store { target, source } => match target {
                StoreTarget::Ptr(addr) => write!(f, "LD [{addr}], {source}"),
                StoreTarget::LiteralPtr(addr) => write!(f, "LD [${addr:04x}], {source}"),
                StoreTarget::SectionAddrPtr { section, offset } => {
                    if *offset == 0 {
                        write!(f, "LD [{}], {source}", section.display(lir))
                    } else {
                        write!(f, "LD [{}+${offset:04x}], {source}", section.display(lir))
                    }
                }
                StoreTarget::HighPtr(addr) => write!(f, "LD [$ff00+{addr}], {source}"),
                StoreTarget::HighLiteralPtr(addr) => write!(f, "LD [$ff00+${addr:02x}], {source}"),
            },
            Instr::StoreValue { target, source } => write!(f, "LD [{target}], {source}"),
            Instr::StoreInc {
                revised,
                left,
                right,
            } => write!(f, "LD [(out={revised};in={left})++], {right}"),
            Instr::StoreDec {
                revised,
                left,
                right,
            } => write!(f, "LD [(out={revised};in={left})--], {right}"),
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
                    "ADC (out={target};in={left}), {right}; z(out={z}), c(out={c};in={c_in})"
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
                    "ADD (out={target};in={left}), {right}; z(out={z}), c(out={c})"
                )
            }
            Instr::Cp { z, c, left, right } => {
                write!(f, "CP (in={left}), {right}; z(out={z}), c(out={c})")
            }
            Instr::Dec { target, z, source } => {
                write!(f, "DEC (out={target};in={source}); z(out={z})")
            }
            Instr::DecPtr { z, addr } => {
                write!(f, "DEC [{addr}]; z(out={z})")
            }
            Instr::Inc { target, z, source } => {
                write!(f, "INC (out={target};in={source}); z(out={z})")
            }
            Instr::IncPtr { z, addr } => {
                write!(f, "INC [{addr}]; z(out={z})")
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
                    "SBC (out={target};in={left}), {right}; z(out={z}), c(out={c};in={c_in})"
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
                    "SUB (out={target};in={left}), {right}; z(out={z}), c(out={c})"
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
                    "AND (out={target};in={left}), {right}; z(out={z}), c(out={c})"
                )
            }
            Instr::Cpl { target, source } => {
                write!(f, "CPL (out={target};in={source})")
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
                    "OR (out={target};in={left}), {right}; z(out={z}), c(out={c})"
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
                    "XOR (out={target};in={left}), {right}; z(out={z}), c(out={c})"
                )
            }
            Instr::Bit { z, bit, source } => {
                write!(f, "BIT {bit}, {source}; z(out={z})")
            }
            Instr::BitPtr { z, bit, addr } => {
                write!(f, "BIT {bit}, [{addr}]; z(out={z})")
            }
            Instr::Res {
                target,
                bit,
                source,
            } => {
                write!(f, "RES {bit}, (out={target};in={source})")
            }
            Instr::ResPtr { bit, addr } => {
                write!(f, "RES {bit}, [{addr}]")
            }
            Instr::Set {
                target,
                bit,
                source,
            } => {
                write!(f, "SET {bit}, (out={target};in={source})")
            }
            Instr::SetPtr { bit, addr } => {
                write!(f, "SET {bit}, [{addr}]")
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
                    "RL (out={target};in={source}); z(out={z}), c(out={c};in={c_in})"
                )
            }
            Instr::RlPtr { z, c, addr, c_in } => {
                write!(f, "RL [{addr}]; z(out={z}), c(out={c};in={c_in})")
            }
            Instr::Rlc {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "RLC (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::RlcPtr { z, c, addr } => {
                write!(f, "RLC [{addr}]; z(out={z}), c(out={c})")
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
                    "RR (out={target};in={source}); z(out={z}), c(out={c};in={c_in})"
                )
            }
            Instr::RrPtr { z, c, addr, c_in } => {
                write!(f, "RR [{addr}]; z(out={z}), c(out={c};in={c_in})")
            }
            Instr::Rrc {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "RRC (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::RrcPtr { z, c, addr } => {
                write!(f, "RRC [{addr}]; z(out={z}), c(out={c})")
            }
            Instr::Sla {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "SLA (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::SlaPtr { z, c, addr } => {
                write!(f, "SLA [{addr}]; z(out={z}), c(out={c})")
            }
            Instr::Sra {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "SRA (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::SraPtr { z, c, addr } => {
                write!(f, "SRA [{addr}]; z(out={z}), c(out={c})")
            }
            Instr::Srl {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "SRL (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::SrlPtr { z, c, addr } => {
                write!(f, "SRL [{addr}]; z(out={z}), c(out={c})")
            }
            Instr::Swap {
                target,
                z,
                c,
                source,
            } => {
                write!(f, "SWAP (out={target};in={source}); z(out={z}), c(out={c})")
            }
            Instr::SwapPtr { z, c, addr } => {
                write!(f, "SRL [{addr}]; z(out={z}), c(out={c})")
            }
            Instr::Ccf { c, c_in } => {
                write!(f, "CCF ; c(out={c};in={c_in})")
            }
            Instr::Scf { c } => {
                write!(f, "SCF ; c(out={c})")
            }
            Instr::Add16 {
                target,
                c,
                left,
                right,
            } => {
                write!(f, "ADD (out={target};in={left}), {right}; c(out={c})")
            }
            Instr::Dec16 { target, source } => {
                write!(f, "DEC (out={target};in={source})")
            }
            Instr::Inc16 { target, source } => {
                write!(f, "INC (out={target};in={source})")
            }
            Instr::Join {
                target,
                high_in,
                low_in,
            } => {
                write!(f, "(out={target};high_in={high_in};low_in={low_in})")
            }
            Instr::Split { high, low, source } => {
                write!(f, "(high_out={high};low_out={low};in={source})")
            }
            Instr::LdZero { target, z_in } => {
                write!(f, "(out={target};z_in={z_in})")
            }
            Instr::LdCarry { target, c_in } => {
                write!(f, "(out={target};c_in={c_in})")
            }
            Instr::LdReturn8 { target } => {
                write!(f, "A(out={target})")
            }
            Instr::LdReturn16 { target } => {
                write!(f, "HL(out={target})")
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
                match source {
                    FlagId::Zero(z) => write!(f, "z(in={z})"),
                    FlagId::Carry(c) => write!(f, "z(in={c})"),
                }?;
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
                write!(f, "CALL {callee}; return={return_block}")
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
                match source {
                    FlagId::Zero(z) => write!(f, "z(in={z})"),
                    FlagId::Carry(c) => write!(f, "z(in={c})"),
                }?;
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
                match source {
                    FlagId::Zero(z) => write!(f, "z(in={z})"),
                    FlagId::Carry(c) => write!(f, "z(in={c})"),
                }?;
                write!(f, "; else={else_block}")
            }
            Terminator::Halt { return_block } => {
                write!(f, "HALT; return={return_block}")
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::From)]
pub enum FlagId {
    Zero(ZeroFlagId),
    Carry(CarryFlagId),
}

impl PartialOrd for FlagId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (FlagId::Carry(this), FlagId::Carry(other)) => Some(Ord::cmp(this, other)),
            (FlagId::Zero(this), FlagId::Zero(other)) => Some(Ord::cmp(this, other)),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum Ld8Source {
    Operand(Operand8),
    LiteralPtr(u16),
    SectionAddrPtr { section: SectionId, offset: u16 },
    HighPtr(R8Id),
    HighLiteralPtr(u8),
}

impl Ld8Source {
    #[must_use]
    pub fn display<'a>(&'a self, lir: &'a Lir) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Ld8Source::Operand(operand) => write!(f, "{operand}"),
            Ld8Source::LiteralPtr(addr) => write!(f, "[${addr:04x}]"),
            Ld8Source::SectionAddrPtr { section, offset } => {
                write!(f, "[{}+${offset:04x}]", section.display(lir))
            }
            Ld8Source::HighPtr(addr) => write!(f, "[$ff00 + {addr}]"),
            Ld8Source::HighLiteralPtr(addr) => write!(f, "[$ff00 + {addr}]"),
        })
    }
}

#[derive(Debug)]
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

#[derive(Debug)]
pub enum StoreTarget {
    Ptr(R16Id),
    LiteralPtr(u16),
    SectionAddrPtr { section: SectionId, offset: u16 },
    HighPtr(R8Id),
    HighLiteralPtr(u8),
}

#[derive(Debug)]
pub enum Operand8 {
    Ptr(R16Id),
    Direct(R8Id),
    Literal(u8),
}

impl fmt::Display for Operand8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand8::Ptr(addr) => write!(f, "[{addr}]"),
            Operand8::Direct(reg) => write!(f, "{reg}"),
            Operand8::Literal(value) => write!(f, "${value:02x}"),
        }
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
    outstanding_phi_r8s: SparseSecondaryMap<R8Id, SparseSlotKeySet<BlockId>>,
    outstanding_phi_r16s: SparseSecondaryMap<R16Id, SparseSlotKeySet<BlockId>>,
    phi_r16s_high_low: SparseSecondaryMap<R16Id, (R8Id, R8Id)>,
}

impl CodeSectionBuilder {
    #[must_use]
    pub fn build(self) -> Section {
        assert!(
            self.outstanding_phi_r8s.is_empty(),
            "uncompleted phi r8 registers"
        );
        assert!(
            self.outstanding_phi_r16s.is_empty(),
            "uncompleted phi r16 registers"
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
        self.data.r8_registers.insert(Register {
            defined: SparseSecondaryMap::new(),
        })
    }

    fn new_r16(&mut self) -> R16Id {
        self.data.r16_registers.insert(Register {
            defined: SparseSecondaryMap::new(),
        })
    }

    fn new_zero_flag(&mut self) -> ZeroFlagId {
        self.data.zero_flags.insert(Register {
            defined: SparseSecondaryMap::new(),
        })
    }

    fn new_carry_flag(&mut self) -> CarryFlagId {
        self.data.carry_flags.insert(Register {
            defined: SparseSecondaryMap::new(),
        })
    }

    fn insert_instr(&mut self, block: BlockId, instr: Instr) {
        let index = self.data.blocks[block].instructions.len();
        for target in instr.target_r8() {
            assert!(!self.data.r8_registers[target].defined.contains_key(block));
            self.data.r8_registers[target].defined.insert(block, index);
        }
        if let Some(target) = instr.target_r16() {
            assert!(!self.data.r16_registers[target].defined.contains_key(block));
            self.data.r16_registers[target].defined.insert(block, index);
        }
        if let Some(flag) = instr.zero_flag() {
            assert!(!self.data.zero_flags[flag].defined.contains_key(block));
            self.data.zero_flags[flag].defined.insert(block, index);
        }
        if let Some(flag) = instr.carry_flag() {
            assert!(!self.data.carry_flags[flag].defined.contains_key(block));
            self.data.carry_flags[flag].defined.insert(block, index);
        }
        self.data.blocks[block].instructions.push(instr);
    }

    #[must_use]
    pub fn reserve_phi_r8(&mut self) -> R8Id {
        let r8 = self.new_r8();
        self.outstanding_phi_r8s.insert(r8, SparseSlotKeySet::new());
        r8
    }

    pub fn insert_phi_r8_ld(&mut self, block: BlockId, target: R8Id, source: R8Id) {
        match self.outstanding_phi_r8s.entry(target).unwrap() {
            Entry::Occupied(mut entry) => assert!(
                entry.get_mut().insert(block),
                "block {block:?} already inserted load for phi register {target:?}"
            ),
            Entry::Vacant(_) => panic!("{target:?} not a phi register"),
        }
        self.insert_instr(
            block,
            Instr::Ld8 {
                target,
                source: Ld8Source::Operand(Operand8::Direct(source)),
            },
        );
    }

    pub fn complete_phi_r8(&mut self, target: R8Id, blocks: &[BlockId]) {
        let actual = self
            .outstanding_phi_r8s
            .remove(target)
            .unwrap_or_else(|| panic!("{target:?} already completed"));
        let expected = blocks.iter().copied().collect::<SparseSlotKeySet<_>>();
        assert_eq!(
            actual, expected,
            "missing phi loads for phi register {target:?}"
        );
    }

    #[must_use]
    pub fn reserve_phi_r16(&mut self) -> R16Id {
        let r16 = self.new_r16();
        self.outstanding_phi_r16s
            .insert(r16, SparseSlotKeySet::new());
        let low = self.new_r8();
        let high = self.new_r8();
        self.phi_r16s_high_low.insert(r16, (high, low));
        r16
    }

    pub fn insert_phi_r16_ld(&mut self, block: BlockId, target: R16Id, source: R16Id) {
        match self.outstanding_phi_r16s.entry(target).unwrap() {
            Entry::Occupied(mut entry) => assert!(
                entry.get_mut().insert(block),
                "block {block:?} already inserted load for phi register {target:?}"
            ),
            Entry::Vacant(_) => panic!("{target:?} not a phi register"),
        }
        let (source_high, source_low) = self.insert_split(block, source);
        let (target_high, target_low) = self.phi_r16s_high_low[target];
        self.insert_instr(
            block,
            Instr::Ld8 {
                target: target_low,
                source: Ld8Source::Operand(Operand8::Direct(source_low)),
            },
        );
        self.insert_instr(
            block,
            Instr::Ld8 {
                target: target_high,
                source: Ld8Source::Operand(Operand8::Direct(source_high)),
            },
        );
        self.insert_instr(
            block,
            Instr::Join {
                target,
                high_in: target_high,
                low_in: target_low,
            },
        );
    }

    pub fn complete_phi_r16(&mut self, target: R16Id, blocks: &[BlockId]) {
        let actual = self
            .outstanding_phi_r16s
            .remove(target)
            .unwrap_or_else(|| panic!("{target:?} already completed"));
        let expected = blocks.iter().copied().collect::<SparseSlotKeySet<_>>();
        assert_eq!(
            actual, expected,
            "missing phi loads for phi register {target:?}"
        );
    }

    /// `LD r8,r8`
    /// `LD r8,n8`
    /// `LD r8,[HL]`
    /// `LD A,[r16]`
    /// `LD A,[n16]`
    /// `LDH A,[n16]`
    /// `LDH A,[C]`
    #[must_use]
    pub fn insert_ld8(&mut self, block: BlockId, source: Ld8Source) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::Ld8 { target, source });
        target
    }

    /// `LD A,[HLI]`
    #[must_use]
    pub fn insert_ld8_inc(&mut self, block: BlockId, source: R16Id) -> (R8Id, R16Id) {
        let target = self.new_r8();
        let revised = self.new_r16();
        self.insert_instr(
            block,
            Instr::Ld8Inc {
                target,
                revised,
                source,
            },
        );
        (target, revised)
    }

    /// `LD A,[HLD]`
    #[must_use]
    pub fn insert_ld8_dec(&mut self, block: BlockId, source: R16Id) -> (R8Id, R16Id) {
        let target = self.new_r8();
        let revised = self.new_r16();
        self.insert_instr(
            block,
            Instr::Ld8Dec {
                target,
                revised,
                source,
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

    /// `LD [HL],r8`
    /// `LD [r16],A`
    /// `LD [n16],A`
    /// `LDH [n16],A`
    /// `LDH [C],A`
    pub fn insert_store(&mut self, block: BlockId, target: StoreTarget, source: R8Id) {
        self.insert_instr(block, Instr::Store { target, source });
    }

    /// `LD [HL],n8`
    pub fn insert_store_value(&mut self, block: BlockId, target: R16Id, source: u8) {
        self.insert_instr(block, Instr::StoreValue { target, source });
    }

    /// `LD [HLI],A`
    #[must_use]
    pub fn insert_store_inc(&mut self, block: BlockId, left: R16Id, right: R8Id) -> R16Id {
        let revised = self.new_r16();
        self.insert_instr(
            block,
            Instr::StoreInc {
                revised,
                left,
                right,
            },
        );
        revised
    }

    /// `LD [HLD],A`
    #[must_use]
    pub fn insert_store_dec(&mut self, block: BlockId, left: R16Id, right: R8Id) -> R16Id {
        let revised = self.new_r16();
        self.insert_instr(
            block,
            Instr::StoreDec {
                revised,
                left,
                right,
            },
        );
        revised
    }

    /// `ADC A,r8`
    /// `ADC A,[HL]`
    /// `ADC A,n8`
    #[must_use]
    pub fn insert_adc(
        &mut self,
        block: BlockId,
        left: R8Id,
        right: Operand8,
        carry: CarryFlagId,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
                c_in: carry,
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
        left: R8Id,
        right: Operand8,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
    pub fn insert_cp(
        &mut self,
        block: BlockId,
        left: R8Id,
        right: Operand8,
    ) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Cp { z, c, left, right });
        (z, c)
    }

    /// `DEC r8`
    #[must_use]
    pub fn insert_dec(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Dec { target, z, source });
        (target, z)
    }

    /// `DEC [HL]`
    #[must_use]
    pub fn insert_dec_ptr(&mut self, block: BlockId, addr: R16Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::DecPtr { z, addr });
        z
    }

    /// `INC r8`
    #[must_use]
    pub fn insert_inc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId) {
        let target = self.new_r8();
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Inc { target, z, source });
        (target, z)
    }

    /// `INC [HL]`
    #[must_use]
    pub fn insert_inc_ptr(&mut self, block: BlockId, addr: R16Id) -> ZeroFlagId {
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
        left: R8Id,
        right: Operand8,
        carry: CarryFlagId,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
                c_in: carry,
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
        left: R8Id,
        right: Operand8,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
        left: R8Id,
        right: Operand8,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
    pub fn insert_cpl(&mut self, block: BlockId, source: R8Id) -> R8Id {
        let target = self.new_r8();
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
        left: R8Id,
        right: Operand8,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
        left: R8Id,
        right: Operand8,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
        let target = self.new_r8();
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
    pub fn insert_bit(&mut self, block: BlockId, bit: u3, source: R8Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Bit { z, bit, source });
        z
    }

    /// `BIT u3,[HL]`
    #[must_use]
    pub fn insert_bit_ptr(&mut self, block: BlockId, bit: u3, addr: R16Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::BitPtr { z, bit, addr });
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
    pub fn insert_res_ptr(&mut self, block: BlockId, bit: u3, addr: R16Id) {
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
    pub fn insert_set_ptr(&mut self, block: BlockId, bit: u3, addr: R16Id) {
        self.insert_instr(block, Instr::SetPtr { bit, addr });
    }

    /// `RL r8`
    /// `RLA`
    #[must_use]
    pub fn insert_rl(
        &mut self,
        block: BlockId,
        source: R8Id,
        c_in: CarryFlagId,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
        addr: R16Id,
        c_in: CarryFlagId,
    ) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RlPtr { z, c, addr, c_in });
        (z, c)
    }

    /// `RLC r8`
    /// `RLCA`
    #[must_use]
    pub fn insert_rlc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_rlc_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
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
        c_in: CarryFlagId,
    ) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
        addr: R16Id,
        c_in: CarryFlagId,
    ) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RrPtr { z, c, addr, c_in });
        (z, c)
    }

    /// `RRC r8`
    /// `RRCA`
    #[must_use]
    pub fn insert_rrc(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_rrc_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RrcPtr { z, c, addr });
        (z, c)
    }

    /// `SLA r8`
    #[must_use]
    pub fn insert_sla(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_sla_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SlaPtr { z, c, addr });
        (z, c)
    }

    /// Shift right arithmetically (signed)
    /// `SRA r8`
    #[must_use]
    pub fn insert_sra(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_sra_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SraPtr { z, c, addr });
        (z, c)
    }

    /// Shift right logically (unsigned)
    /// `SRL r8`
    #[must_use]
    pub fn insert_srl(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_srl_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SrlPtr { z, c, addr });
        (z, c)
    }

    /// `SWAP r8`
    #[must_use]
    pub fn insert_swap(&mut self, block: BlockId, source: R8Id) -> (R8Id, ZeroFlagId, CarryFlagId) {
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
    pub fn insert_swap_ptr(&mut self, block: BlockId, addr: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SwapPtr { z, c, addr });
        (z, c)
    }

    /// CCF
    #[must_use]
    pub fn insert_ccf(&mut self, block: BlockId, c_in: CarryFlagId) -> CarryFlagId {
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Ccf { c, c_in });
        c
    }

    /// SCF
    #[must_use]
    pub fn insert_scf(&mut self, block: BlockId) -> CarryFlagId {
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::Scf { c });
        c
    }

    /// `ADD HL,r16`
    #[must_use]
    pub fn insert_add16(
        &mut self,
        block: BlockId,
        left: R16Id,
        right: R16Id,
    ) -> (R16Id, CarryFlagId) {
        let target = self.new_r16();
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
        let low = self.new_r8();
        let high = self.new_r8();
        self.insert_instr(block, Instr::Split { high, low, source });
        (high, low)
    }

    #[must_use]
    pub fn insert_join(&mut self, block: BlockId, high_in: R8Id, low_in: R8Id) -> R16Id {
        let target = self.new_r16();
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

    #[must_use]
    pub fn insert_ld_zero(&mut self, block: BlockId, z_in: ZeroFlagId) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::LdZero { target, z_in });
        target
    }

    #[must_use]
    pub fn insert_ld_carry(&mut self, block: BlockId, c_in: CarryFlagId) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::LdCarry { target, c_in });
        target
    }

    #[must_use]
    pub fn insert_ld_return_8(&mut self, block: BlockId) -> R8Id {
        let target = self.new_r8();
        self.insert_instr(block, Instr::LdReturn8 { target });
        target
    }

    #[must_use]
    pub fn insert_ld_return_16(&mut self, block: BlockId) -> R16Id {
        let target = self.new_r16();
        self.insert_instr(block, Instr::LdReturn16 { target });
        target
    }
}
