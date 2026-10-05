#[derive(Debug, Clone, PartialEq)]
pub struct ObjFile {
    pub revision: u32,
    pub source_nodes: Vec<SourceNode>,
    pub symbols: Vec<Symbol>,
    pub sections: Vec<Section>,
    pub assertions: Vec<Assertion>,
}

impl Default for ObjFile {
    fn default() -> Self {
        ObjFile {
            revision: 13,
            source_nodes: Vec::new(),
            symbols: Vec::new(),
            sections: Vec::new(),
            assertions: Vec::new(),
        }
    }
}

impl ObjFile {
    pub fn insert_source_node(&mut self, node: SourceNode) -> SourceNodeId {
        let id = SourceNodeId(
            u32::try_from(self.source_nodes.len()).expect("source node count should fit in a u32"),
        );
        self.source_nodes.push(node);
        id
    }

    pub fn insert_symbol(&mut self, symbol: Symbol) -> SymbolId {
        let id =
            SymbolId(u32::try_from(self.symbols.len()).expect("symbol count should fit in a u32"));
        self.symbols.push(symbol);
        id
    }

    pub fn insert_section(&mut self, section: Section) -> SectionId {
        let id = SectionId(
            u32::try_from(self.sections.len()).expect("section count should fit in a u32"),
        );
        self.sections.push(section);
        id
    }

    pub fn insert_assertion(&mut self, assertion: Assertion) -> AssertionId {
        let id = AssertionId(
            u32::try_from(self.assertions.len()).expect("assertion count should fit in a u32"),
        );
        self.assertions.push(assertion);
        id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceNodeId(pub(crate) u32);

impl SourceNodeId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceNode {
    pub parent: Option<SourceNodeId>,
    pub parent_line_no: u32,
    pub data: SourceNodeData,
    pub quieted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceNodeData {
    Repeat { iterations: Vec<u32> },
    File { name: String },
    Macro { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub(crate) u32);

impl SymbolId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub data: SymbolData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolData {
    Local(SymbolDefinition),
    Import,
    Exported(SymbolDefinition),
}

impl SymbolData {
    #[must_use]
    pub fn as_definition(&self) -> Option<&SymbolDefinition> {
        match self {
            SymbolData::Local(definition) | SymbolData::Exported(definition) => Some(definition),
            SymbolData::Import => None,
        }
    }

    pub fn as_definition_mut(&mut self) -> Option<&mut SymbolDefinition> {
        match self {
            SymbolData::Local(definition) | SymbolData::Exported(definition) => Some(definition),
            SymbolData::Import => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolDefinition {
    pub node: SourceNodeId,
    pub line_no: u32,
    pub section: Option<SectionId>,
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionId(pub(crate) u32);

impl SectionId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: String,
    pub node: SourceNodeId,
    pub line_no: u32,
    pub data: SectionData,
    pub mode: LinkingMode,
    pub address: Option<u32>,
    pub bank: Option<u32>,
    pub alignment: u8,
    pub alignment_offset: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SectionData {
    WRam0(RamSectionData),
    VRam(RamSectionData),
    RomX(RomSectionData),
    Rom0(RomSectionData),
    HRam(RamSectionData),
    WRamX(RamSectionData),
    SRam(RamSectionData),
    Oam(RamSectionData),
}

impl SectionData {
    #[must_use]
    pub fn as_ram_data(&self) -> Option<&RamSectionData> {
        match self {
            SectionData::WRam0(data)
            | SectionData::VRam(data)
            | SectionData::HRam(data)
            | SectionData::WRamX(data)
            | SectionData::SRam(data)
            | SectionData::Oam(data) => Some(data),
            SectionData::RomX(_) | SectionData::Rom0(_) => None,
        }
    }
    pub fn as_ram_data_mut(&mut self) -> Option<&mut RamSectionData> {
        match self {
            SectionData::WRam0(data)
            | SectionData::VRam(data)
            | SectionData::HRam(data)
            | SectionData::WRamX(data)
            | SectionData::SRam(data)
            | SectionData::Oam(data) => Some(data),
            SectionData::RomX(_) | SectionData::Rom0(_) => None,
        }
    }

    #[must_use]
    pub fn as_rom_data(&self) -> Option<&RomSectionData> {
        match self {
            SectionData::RomX(data) | SectionData::Rom0(data) => Some(data),
            SectionData::WRam0(_)
            | SectionData::VRam(_)
            | SectionData::HRam(_)
            | SectionData::WRamX(_)
            | SectionData::SRam(_)
            | SectionData::Oam(_) => None,
        }
    }
    pub fn as_rom_data_mut(&mut self) -> Option<&mut RomSectionData> {
        match self {
            SectionData::RomX(data) | SectionData::Rom0(data) => Some(data),
            SectionData::WRam0(_)
            | SectionData::VRam(_)
            | SectionData::HRam(_)
            | SectionData::WRamX(_)
            | SectionData::SRam(_)
            | SectionData::Oam(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    WRam0,
    VRam,
    RomX,
    Rom0,
    HRam,
    WRamX,
    SRam,
    Oam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkingMode {
    Normal,
    Union,
    Fragment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RomSectionData {
    pub bytes: Vec<u8>,
    pub patches: Vec<SectionPatch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RamSectionData {
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionPatch {
    pub node: SourceNodeId,
    pub line_no: u32,
    /// offset within the section to apply the patch
    pub offset: u32,
    /// id of the section in which PC is located.
    ///
    /// Usually the current section, but can be another section for RAM code
    pub pc_section_id: SectionId,
    /// offset within the section for which the instruction being patched is.
    ///
    /// Usually this will be 1 less than offset, or 2 less for CB prefixed
    /// instructions.
    pub pc_offset: u32,
    pub kind: SectionPatchKind,
    pub instructions: Vec<PatchInstr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SectionPatchKind {
    Byte1,
    Byte2,
    Byte4,
    Jr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchInstr {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Exp,
    BitOr,
    BitAnd,
    BitXor,
    Cmp,
    And,
    Or,
    Not,
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    ShiftLeft,
    SignedShiftRight,
    UnsignedShiftRight,
    SymbolBank(SymbolId),
    SectionBank(String),
    PcBank,
    SizeOfSection(String),
    StartOfSection(String),
    SizeOfType(SectionKind),
    StartOfType(SectionKind),
    LdhCheck,
    RstCheck,
    BitCheck(u8),
    High,
    Low,
    BitWidth,
    TrailingZeros,
    Int(u32),
    SymbolValue(SymbolId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssertionId(pub(crate) u32);

impl AssertionId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub node: SourceNodeId,
    pub line_no: u32,
    pub offset: u32,
    pub pc_section_id: SectionId,
    pub pc_offset: u32,
    pub kind: AssertionKind,
    pub instructions: Vec<PatchInstr>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssertionKind {
    Warning,
    Error,
    Fatal,
}
