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
use indexmap::IndexMap;
use indexmap::map::Entry;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap, new_key_type};
use std::assert_matches;
use std::cmp::Ordering;
use std::num::NonZeroU16;

new_key_type! { pub struct SectionId; }
new_key_type! { pub struct BlockId; }
new_key_type! { pub struct ZeroFlagId; }
new_key_type! { pub struct CarryFlagId; }
new_key_type! { pub struct R8Id; }
new_key_type! { pub struct R16Id; }

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

#[derive(Debug, Default)]
pub struct Lir {
    pub sections: SlotMap<SectionId, Section>,
}

#[derive(Debug)]
pub struct Section {
    pub data: SectionData,
}

impl Section {
    pub fn region(&self) -> Option<RegionKind> {
        match &self.data {
            SectionData::Code(CodeSectionData { region, .. })
            | SectionData::Data(DataSectionData { region, .. }) => region.map(RegionKind::Rom),
            SectionData::Ram(RamSectionData { region, .. }) => Some(RegionKind::Ram(*region)),
        }
    }

    #[must_use]
    pub fn address(&self) -> Option<u16> {
        match &self.data {
            SectionData::Code(CodeSectionData { address, .. })
            | SectionData::Data(DataSectionData { address, .. }) => *address,
            SectionData::Ram(RamSectionData { address, .. }) => Some(*address),
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

#[derive(Debug, Clone, Copy)]
pub enum RegionKind {
    Rom(RomRegionKind),
    Ram(RamRegionKind),
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
    pub address: Option<u16>,
    pub blocks: SlotMap<BlockId, Block>,
    pub functions: Box<IndexMap<Identifier, BlockId>>,
    pub zero_flags: SlotMap<ZeroFlagId, Register>,
    pub carry_flags: SlotMap<CarryFlagId, Register>,
    pub r8_registers: SlotMap<R8Id, Register>,
    pub r16_registers: SlotMap<R16Id, Register>,
}

#[derive(Debug, Clone, Copy)]
pub enum RomRegionKind {
    Rom0,
    RomX(Option<NonZeroU16>),
}

#[derive(Debug)]
pub struct DataSectionData {
    pub region: Option<RomRegionKind>,
    pub address: Option<u16>,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct RamSectionData {
    pub region: RamRegionKind,
    pub address: u16,
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

/// LIR Basic block
#[derive(Debug)]
pub struct Block {
    pub instructions: Vec<Instr>,
    pub terminator: Terminator,
    previous: Vec<BlockId>,
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
    Ld16 { target: R16Id, source: u16 },
    /// `LD [HL],r8`
    /// `LD [r16],A`
    /// `LD [n16],A`
    /// `LDH [n16],A`
    /// `LDH [C],A`
    Store { target: StoreTarget, source: R8Id },
    /// `LD [HL],n8`
    StoreValue { target: R16Id, source: u16 },
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
    DecPtr { z: ZeroFlagId, pointer: R16Id },
    /// `INC r8`
    Inc {
        target: R8Id,
        z: ZeroFlagId,
        source: R8Id,
    },
    /// `INC [HL]`
    IncPtr { z: ZeroFlagId, pointer: R16Id },
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
        bit: u8,
        source: R8Id,
    },
    /// `BIT u3,[HL]`
    BitPtr {
        z: ZeroFlagId,
        bit: u8,
        source_pointer: R16Id,
    },
    /// `RES u3,r8`
    Res { target: R8Id, bit: u8, source: R8Id },
    /// `RES u3,[HL]`
    ResPtr { bit: u8, pointer: R16Id },
    /// `SET u3,r8`
    Set { target: R8Id, bit: u8, source: R8Id },
    /// `SET u3,[HL]`
    SetPtr { bit: u8, pointer: R16Id },
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
        pointer: R16Id,
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
}

impl Instr {
    #[must_use]
    pub fn target_r8(&self) -> Option<R8Id> {
        match self {
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
            | Instr::Swap { target, .. } => Some(*target),
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
            | Instr::Inc16 { .. } => None,
        }
    }

    #[must_use]
    pub fn target_r16(&self) -> Option<R16Id> {
        match self {
            Instr::Add16 { target, .. }
            | Instr::Dec16 { target, .. }
            | Instr::Inc16 { target, .. } => Some(*target),
            Instr::Nop
            | Instr::Ld8 { .. }
            | Instr::Ld8Inc { .. }
            | Instr::Ld8Dec { .. }
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
            | Instr::StoreInc { .. }
            | Instr::StoreDec { .. }
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
            | Instr::Scf { .. } => None,
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
            | Instr::Inc16 { .. } => None,
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
            | Instr::Inc16 { .. } => None,
        }
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
    /// `HALT`
    Halt { return_block: BlockId },
    /// `JP n16`
    Jp { block: BlockId },
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
}

#[derive(Debug)]
pub enum Ld8Source {
    Operand(Operand8),
    PtrLiteral(u16),
    HighPtr(R8Id),
    HighPtrLiteral(u8),
}

#[derive(Debug)]
pub enum StoreTarget {
    Ptr(R16Id),
    PtrLiteral(u16),
    HighPtr(R8Id),
    HighPtrLiteral(u8),
}

#[derive(Debug)]
pub enum Operand8 {
    Ptr(R16Id),
    Direct(R8Id),
    Literal(u8),
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
    terminated_blocks: SecondaryMap<BlockId, ()>,
}

impl CodeSectionBuilder {
    #[must_use]
    pub fn build(self) -> Section {
        Section {
            data: SectionData::Code(self.data),
        }
    }
}

impl CodeSectionBuilder {
    #[must_use]
    pub fn new_function(&mut self, name: Identifier) -> BlockId {
        let block = self.data.blocks.insert(Block {
            instructions: Vec::new(),
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        match self.data.functions.entry(name) {
            Entry::Occupied(entry) => {
                panic!("{} already defined", entry.key());
            }
            Entry::Vacant(entry) => {
                entry.insert(block);
            }
        }
        block
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
            !self.terminated_blocks.contains_key(block),
            "block {block:?} is already terminated"
        );
        match &terminator {
            Terminator::Ret => {}
            Terminator::Call { return_block, .. }
            | Terminator::Halt { return_block, .. }
            | Terminator::Jp {
                block: return_block,
                ..
            }
            | Terminator::RetIf {
                else_block: return_block,
                ..
            } => {
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
        self.terminated_blocks.insert(block, ());
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
        if let Some(target) = instr.target_r8() {
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
    pub fn insert_ld16(&mut self, block: BlockId, source: u16) -> R16Id {
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
    pub fn insert_store_value(&mut self, block: BlockId, target: R16Id, source: u16) {
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
    pub fn insert_dec_ptr(&mut self, block: BlockId, pointer: R16Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::DecPtr { z, pointer });
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
    pub fn insert_inc_ptr(&mut self, block: BlockId, pointer: R16Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::IncPtr { z, pointer });
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
    pub fn insert_bit(&mut self, block: BlockId, bit: u8, source: R8Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(block, Instr::Bit { z, bit, source });
        z
    }

    /// `BIT u3,[HL]`
    #[must_use]
    pub fn insert_bit_ptr(&mut self, block: BlockId, bit: u8, source_pointer: R16Id) -> ZeroFlagId {
        let z = self.new_zero_flag();
        self.insert_instr(
            block,
            Instr::BitPtr {
                z,
                bit,
                source_pointer,
            },
        );
        z
    }

    /// `RES u3,r8`
    #[must_use]
    pub fn insert_res(&mut self, block: BlockId, bit: u8, source: R8Id) -> R8Id {
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
    pub fn insert_res_ptr(&mut self, block: BlockId, bit: u8, pointer: R16Id) {
        self.insert_instr(block, Instr::ResPtr { bit, pointer });
    }

    /// `SET u3,r8`
    #[must_use]
    pub fn insert_set(&mut self, block: BlockId, bit: u8, source: R8Id) -> R8Id {
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
    pub fn insert_set_ptr(&mut self, block: BlockId, bit: u8, pointer: R16Id) {
        self.insert_instr(block, Instr::SetPtr { bit, pointer });
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
        pointer: R16Id,
        c_in: CarryFlagId,
    ) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::RlPtr {
                z,
                c,
                pointer,
                c_in,
            },
        );
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
    pub fn insert_rlc_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RlcPtr { z, c, pointer });
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
        pointer: R16Id,
        c_in: CarryFlagId,
    ) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(
            block,
            Instr::RrPtr {
                z,
                c,
                pointer,
                c_in,
            },
        );
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
    pub fn insert_rrc_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::RrcPtr { z, c, pointer });
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
    pub fn insert_sla_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SlaPtr { z, c, pointer });
        (z, c)
    }

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

    /// `SRA [HL]`
    #[must_use]
    pub fn insert_sra_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SraPtr { z, c, pointer });
        (z, c)
    }

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

    /// `SRL [HL]`
    #[must_use]
    pub fn insert_srl_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SrlPtr { z, c, pointer });
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
    pub fn insert_swap_ptr(&mut self, block: BlockId, pointer: R16Id) -> (ZeroFlagId, CarryFlagId) {
        let z = self.new_zero_flag();
        let c = self.new_carry_flag();
        self.insert_instr(block, Instr::SwapPtr { z, c, pointer });
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
}
