use crate::{
    Assertion, AssertionKind, LinkingMode, ObjFile, PatchInstr, RamSectionData, RomSectionData,
    Section, SectionData, SectionId, SectionKind, SectionPatch, SectionPatchKind, SourceNode,
    SourceNodeData, SourceNodeId, Symbol, SymbolData, SymbolDefinition, SymbolId,
};
use std::collections::VecDeque;
use std::io;
use std::io::Read;

trait ObjFileReader: io::BufRead {
    fn read_byte(&mut self) -> io::Result<u8> {
        let mut bytes = [0u8; 1];
        self.read_exact(&mut bytes)?;
        Ok(bytes[0])
    }

    fn read_long(&mut self) -> io::Result<u32> {
        let mut bytes = [0u8; 4];
        self.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_optional_long(&mut self) -> io::Result<Option<u32>> {
        let value = self.read_long()?;
        if value == u32::MAX {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn read_string(&mut self) -> io::Result<String> {
        let mut bytes = Vec::new();
        self.read_until(b'\0', &mut bytes)?;
        bytes.pop();
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn read_count(&mut self) -> io::Result<usize> {
        let count = self.read_long()?;
        if count > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "must be positive i32",
            ));
        }
        Ok(count as usize)
    }

    fn read_optional_node_id(&mut self) -> io::Result<Option<SourceNodeId>> {
        let id = self.read_long()?;
        if id == u32::MAX {
            return Ok(None);
        }
        if id > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "node id must be positive i32",
            ));
        }
        Ok(Some(SourceNodeId(id)))
    }

    fn read_node_id(&mut self) -> io::Result<SourceNodeId> {
        let id = self.read_long()?;
        if id > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "node id must be positive i32",
            ));
        }
        Ok(SourceNodeId(id))
    }

    fn read_symbol_id(&mut self) -> io::Result<SymbolId> {
        let id = self.read_long()?;
        if id > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "symbol id must be positive i32",
            ));
        }
        Ok(SymbolId(id))
    }

    fn read_optional_section_id(&mut self) -> io::Result<Option<SectionId>> {
        let id = self.read_long()?;
        if id == u32::MAX {
            return Ok(None);
        }
        if id > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "section id must be positive i32",
            ));
        }
        Ok(Some(SectionId(id)))
    }

    fn read_section_id(&mut self) -> io::Result<SectionId> {
        let id = self.read_long()?;
        if id > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "section id must be positive i32",
            ));
        }
        Ok(SectionId(id))
    }

    fn read_obj_file(&mut self) -> io::Result<ObjFile> {
        let mut magic = [0u8; 4];
        self.read_exact(&mut magic)?;
        if &magic != b"RGB9" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an RGBDS object file",
            ));
        }
        let revision = self.read_long()?;
        let symbols_count = self.read_count()?;
        let sections_count = self.read_count()?;
        let nodes_count = self.read_count()?;

        let mut source_nodes = VecDeque::with_capacity(nodes_count);
        for _ in 0..nodes_count {
            source_nodes.push_front(self.read_source_node()?);
        }
        let source_nodes = Vec::from(source_nodes);

        let mut symbols = Vec::with_capacity(symbols_count);
        for _ in 0..symbols_count {
            symbols.push(self.read_symbol()?);
        }

        let mut sections = Vec::with_capacity(sections_count);
        for _ in 0..sections_count {
            sections.push(self.read_section()?);
        }

        let assertions_count = self.read_count()?;
        let mut assertions = Vec::with_capacity(assertions_count);
        for _ in 0..assertions_count {
            assertions.push(self.read_assertion()?);
        }

        Ok(ObjFile {
            revision,
            source_nodes,
            symbols,
            sections,
            assertions,
        })
    }

    fn read_source_node(&mut self) -> io::Result<SourceNode> {
        let parent = self.read_optional_node_id()?;
        let parent_line_no = self.read_long()?;
        let type_and_quieted_bits = self.read_byte()?;
        let type_bits = type_and_quieted_bits & 0b0111_1111;
        let quieted = type_and_quieted_bits & 0b1000_0000 != 0;
        let data = match type_bits {
            0 => {
                let count = self.read_count()?;
                let mut iterations = Vec::with_capacity(count);
                for _ in 0..count {
                    iterations.push(self.read_long()?);
                }
                SourceNodeData::Repeat { iterations }
            }
            1 => SourceNodeData::File {
                name: self.read_string()?,
            },
            2 => SourceNodeData::Macro {
                name: self.read_string()?,
            },
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected node type {unexpected}"),
                ));
            }
        };
        Ok(SourceNode {
            parent,
            parent_line_no,
            data,
            quieted,
        })
    }

    fn read_symbol(&mut self) -> io::Result<Symbol> {
        let name = self.read_string()?;
        let type_bits = self.read_byte()?;
        let data = match type_bits {
            0 => SymbolData::Local(self.read_symbol_definition()?),
            1 => SymbolData::Import,
            2 => SymbolData::Exported(self.read_symbol_definition()?),
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected symbol type {unexpected}"),
                ));
            }
        };
        Ok(Symbol { name, data })
    }

    fn read_symbol_definition(&mut self) -> io::Result<SymbolDefinition> {
        let node = self.read_node_id()?;
        let line_no = self.read_long()?;
        let section = self.read_optional_section_id()?;
        let value = self.read_long()?;
        Ok(SymbolDefinition {
            node,
            line_no,
            section,
            value,
        })
    }

    fn read_section(&mut self) -> io::Result<Section> {
        let name = self.read_string()?;
        let node = self.read_node_id()?;
        let line_no = self.read_long()?;
        let size = self.read_count()?;
        let type_and_mode_bits = self.read_byte()?;
        let type_bits = type_and_mode_bits & 0b0011_1111;
        let mode_bits = (type_and_mode_bits & 0b1100_0000) >> 6;
        let mode = match mode_bits {
            0b00 => LinkingMode::Normal,
            0b10 => LinkingMode::Union,
            0b01 => LinkingMode::Fragment,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unionized sections and section fragments are mutually exclusive",
                ));
            }
        };
        let address = self.read_optional_long()?;
        let bank = self.read_optional_long()?;
        let alignment = self.read_byte()?;
        let alignment_offset = self.read_long()?;
        let ram_size = u32::try_from(size).expect("section size should fit in a u32");
        let data = match type_bits {
            0 => SectionData::WRam0(RamSectionData { size: ram_size }),
            1 => SectionData::VRam(RamSectionData { size: ram_size }),
            2 => SectionData::RomX(self.read_rom_section_data(size)?),
            3 => SectionData::Rom0(self.read_rom_section_data(size)?),
            4 => SectionData::HRam(RamSectionData { size: ram_size }),
            5 => SectionData::WRamX(RamSectionData { size: ram_size }),
            6 => SectionData::SRam(RamSectionData { size: ram_size }),
            7 => SectionData::Oam(RamSectionData { size: ram_size }),
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected section type {unexpected}"),
                ));
            }
        };
        Ok(Section {
            name,
            node,
            line_no,
            data,
            mode,
            address,
            bank,
            alignment,
            alignment_offset,
        })
    }

    fn read_section_kind(&mut self) -> io::Result<SectionKind> {
        let type_and_mode_bits = self.read_byte()?;
        let type_bits = type_and_mode_bits & 0b0011_1111;
        Ok(match type_bits {
            0 => SectionKind::WRam0,
            1 => SectionKind::VRam,
            2 => SectionKind::RomX,
            3 => SectionKind::Rom0,
            4 => SectionKind::HRam,
            5 => SectionKind::WRamX,
            6 => SectionKind::SRam,
            7 => SectionKind::Oam,
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected section type {unexpected}"),
                ));
            }
        })
    }

    fn read_rom_section_data(&mut self, size: usize) -> io::Result<RomSectionData> {
        let mut bytes = vec![0u8; size];
        self.read_exact(&mut bytes)?;
        let patches_count = self.read_count()?;
        let mut patches = Vec::with_capacity(patches_count);
        for _ in 0..patches_count {
            patches.push(self.read_section_patch()?);
        }
        Ok(RomSectionData { bytes, patches })
    }

    fn read_section_patch(&mut self) -> io::Result<SectionPatch> {
        let node = self.read_node_id()?;
        let line_no = self.read_long()?;
        let offset = self.read_long()?;
        let pc_section_id = self.read_section_id()?;
        let pc_offset = self.read_long()?;
        let kind_bits = self.read_byte()?;
        let kind = match kind_bits {
            0 => SectionPatchKind::Byte1,
            1 => SectionPatchKind::Byte2,
            2 => SectionPatchKind::Byte4,
            3 => SectionPatchKind::Jr,
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected patch type {unexpected}"),
                ));
            }
        };
        let instructions_byte_count = self.read_count()?;
        let mut instructions_read = self.take(instructions_byte_count as u64);
        let mut instructions = Vec::new();
        while let Some(instruction) = instructions_read.read_patch_instr()? {
            instructions.push(instruction);
        }
        Ok(SectionPatch {
            node,
            line_no,
            offset,
            pc_section_id,
            pc_offset,
            kind,
            instructions,
        })
    }

    fn read_patch_instr(&mut self) -> io::Result<Option<PatchInstr>> {
        let type_bits = match self.read_byte() {
            Err(e) => {
                if e.kind() != io::ErrorKind::UnexpectedEof {
                    return Err(e);
                }
                return Ok(None);
            }
            Ok(type_bits) => type_bits,
        };
        Ok(Some(match type_bits {
            0x00 => PatchInstr::Add,
            0x01 => PatchInstr::Sub,
            0x02 => PatchInstr::Mul,
            0x03 => PatchInstr::Div,
            0x04 => PatchInstr::Mod,
            0x05 => PatchInstr::Neg,
            0x06 => PatchInstr::Exp,
            0x10 => PatchInstr::BitOr,
            0x11 => PatchInstr::BitAnd,
            0x12 => PatchInstr::BitXor,
            0x13 => PatchInstr::Cmp,
            0x21 => PatchInstr::And,
            0x22 => PatchInstr::Or,
            0x23 => PatchInstr::Not,
            0x30 => PatchInstr::Eq,
            0x31 => PatchInstr::Ne,
            0x32 => PatchInstr::Gt,
            0x33 => PatchInstr::Lt,
            0x34 => PatchInstr::Ge,
            0x35 => PatchInstr::Le,
            0x40 => PatchInstr::ShiftLeft,
            0x41 => PatchInstr::SignedShiftRight,
            0x42 => PatchInstr::UnsignedShiftRight,
            0x50 => PatchInstr::SymbolBank(self.read_symbol_id()?),
            0x51 => PatchInstr::SectionBank(self.read_string()?),
            0x52 => PatchInstr::PcBank,
            0x53 => PatchInstr::SizeOfSection(self.read_string()?),
            0x54 => PatchInstr::StartOfSection(self.read_string()?),
            0x55 => PatchInstr::SizeOfType(self.read_section_kind()?),
            0x56 => PatchInstr::StartOfType(self.read_section_kind()?),
            0x60 => PatchInstr::LdhCheck,
            0x61 => PatchInstr::RstCheck,
            0x62 => PatchInstr::BitCheck(self.read_byte()?),
            0x70 => PatchInstr::High,
            0x71 => PatchInstr::Low,
            0x72 => PatchInstr::BitWidth,
            0x73 => PatchInstr::TrailingZeros,
            0x80 => PatchInstr::Int(self.read_long()?),
            0x81 => PatchInstr::SymbolValue(self.read_symbol_id()?),
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected RPN expressions type {unexpected}"),
                ));
            }
        }))
    }

    fn read_assertion(&mut self) -> io::Result<Assertion> {
        let node = self.read_node_id()?;
        let line_no = self.read_long()?;
        let offset = self.read_long()?;
        let pc_section_id = self.read_section_id()?;
        let pc_offset = self.read_long()?;
        let kind_bits = self.read_byte()?;
        let kind = match kind_bits {
            0 => AssertionKind::Warning,
            1 => AssertionKind::Error,
            2 => AssertionKind::Fatal,
            unexpected => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected assertion type {unexpected}"),
                ));
            }
        };
        let instructions_byte_count = self.read_count()?;
        let mut instructions_read = self.take(instructions_byte_count as u64);
        let mut instructions = Vec::new();
        while let Some(instruction) = instructions_read.read_patch_instr()? {
            instructions.push(instruction);
        }
        let message = self.read_string()?;
        Ok(Assertion {
            node,
            line_no,
            offset,
            pc_section_id,
            pc_offset,
            kind,
            instructions,
            message,
        })
    }
}

impl<R: io::BufRead> ObjFileReader for R {}

pub fn read(mut read: impl io::BufRead) -> io::Result<ObjFile> {
    read.read_obj_file()
}
