use crate::{
    Assertion, AssertionKind, LinkingMode, ObjFile, PatchInstr, Section, SectionData, SectionKind,
    SectionPatch, SectionPatchKind, SourceNode, SourceNodeData, Symbol, SymbolData,
};
use std::io;
use std::io::Write;

pub(crate) trait ObjFileWriter: Write {
    fn write_byte(&mut self, value: u8) -> io::Result<()> {
        self.write_all(&[value])
    }

    fn write_long(&mut self, value: u32) -> io::Result<()> {
        if value > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{value} is too large"),
            ));
        }
        let bytes = value.to_le_bytes();
        self.write_all(&bytes)
    }

    fn write_optional_long(&mut self, value: Option<u32>) -> io::Result<()> {
        let value = match value {
            None => u32::MAX,
            Some(value) if value > i32::MAX as u32 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{value} is too large"),
                ));
            }
            Some(value) => value,
        };
        let bytes = value.to_le_bytes();
        self.write_all(&bytes)
    }

    fn write_count(&mut self, value: usize) -> io::Result<()> {
        self.write_long(value as u32)
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        if value.as_bytes().contains(&b'\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{value} contains a null byte"),
            ));
        }
        self.write_all(value.as_bytes())?;
        self.write_byte(b'\0')
    }

    fn write_obj_file(&mut self, obj_file: &ObjFile) -> io::Result<()> {
        self.write_all(b"RGB9")?;
        self.write_long(obj_file.revision)?;
        self.write_count(obj_file.symbols.len())?;
        self.write_count(obj_file.sections.len())?;
        self.write_count(obj_file.source_nodes.len())?;
        for node in obj_file.source_nodes.iter().rev() {
            self.write_source_node(node)?;
        }
        for symbol in &obj_file.symbols {
            self.write_symbol(symbol)?;
        }
        for section in &obj_file.sections {
            self.write_section(section)?;
        }
        self.write_count(obj_file.assertions.len())?;
        for assertion in &obj_file.assertions {
            self.write_assertion(assertion)?;
        }
        Ok(())
    }

    fn write_source_node(&mut self, node: &SourceNode) -> io::Result<()> {
        self.write_optional_long(node.parent.map(|id| id.0))?;
        self.write_long(node.parent_line_no)?;
        let mut type_and_quieted_bits = match &node.data {
            SourceNodeData::Repeat { .. } => 0,
            SourceNodeData::File { .. } => 1,
            SourceNodeData::Macro { .. } => 2,
        };
        type_and_quieted_bits |= (node.quieted as u8) << 7;
        self.write_byte(type_and_quieted_bits)?;
        match &node.data {
            SourceNodeData::Repeat { iterations } => {
                self.write_count(iterations.len())?;
                for iteration in iterations {
                    self.write_long(*iteration)?;
                }
                Ok(())
            }
            SourceNodeData::File { name } | SourceNodeData::Macro { name } => {
                self.write_string(name)
            }
        }?;
        Ok(())
    }

    fn write_symbol(&mut self, symbol: &Symbol) -> io::Result<()> {
        self.write_string(&symbol.name)?;
        let type_bits = match &symbol.data {
            SymbolData::Local(_) => 0,
            SymbolData::Import => 1,
            SymbolData::Exported(_) => 2,
        };
        self.write_byte(type_bits)?;
        match &symbol.data {
            SymbolData::Import => {}
            SymbolData::Local(definition) | SymbolData::Exported(definition) => {
                self.write_long(definition.node.0)?;
                self.write_long(definition.line_no)?;
                self.write_optional_long(definition.section.map(|id| id.0))?;
                self.write_long(definition.value)?;
            }
        }
        Ok(())
    }

    fn write_section(&mut self, section: &Section) -> io::Result<()> {
        self.write_string(&section.name)?;
        self.write_long(section.node.0)?;
        self.write_long(section.line_no)?;
        let (mut type_and_mode_bits, size) = match &section.data {
            SectionData::WRam0(data) => (0, data.size as usize),
            SectionData::VRam(data) => (1, data.size as usize),
            SectionData::RomX(data) => (2, data.bytes.len()),
            SectionData::Rom0(data) => (3, data.bytes.len()),
            SectionData::HRam(data) => (4, data.size as usize),
            SectionData::WRamX(data) => (5, data.size as usize),
            SectionData::SRam(data) => (6, data.size as usize),
            SectionData::Oam(data) => (7, data.size as usize),
        };
        type_and_mode_bits |= match &section.mode {
            LinkingMode::Normal => 0b00,
            LinkingMode::Union => 0b10,
            LinkingMode::Fragment => 0b01,
        } << 6;
        self.write_count(size)?;
        self.write_byte(type_and_mode_bits)?;
        self.write_optional_long(section.address)?;
        self.write_optional_long(section.bank)?;
        self.write_byte(section.alignment)?;
        self.write_long(section.alignment_offset)?;
        match &section.data {
            SectionData::WRam0(_)
            | SectionData::VRam(_)
            | SectionData::HRam(_)
            | SectionData::WRamX(_)
            | SectionData::SRam(_)
            | SectionData::Oam(_) => {}
            SectionData::RomX(data) | SectionData::Rom0(data) => {
                self.write_all(&data.bytes)?;
                self.write_count(data.patches.len())?;
                for patch in &data.patches {
                    self.write_section_patch(patch)?;
                }
            }
        }
        Ok(())
    }

    fn write_section_patch(&mut self, patch: &SectionPatch) -> io::Result<()> {
        self.write_long(patch.node.0)?;
        self.write_long(patch.line_no)?;
        self.write_long(patch.offset)?;
        self.write_long(patch.pc_section_id.0)?;
        self.write_long(patch.pc_offset)?;
        let type_bits = match patch.kind {
            SectionPatchKind::Byte1 => 0,
            SectionPatchKind::Byte2 => 1,
            SectionPatchKind::Byte4 => 2,
            SectionPatchKind::Jr => 3,
        };
        self.write_byte(type_bits)?;
        // we have to buffer writes for instructions first as the size is serialized
        // as the number of bytes, not the number of instructions
        let mut instructions_bytes = Vec::<u8>::new();
        for instruction in &patch.instructions {
            instructions_bytes.write_patch_instr(instruction)?;
        }
        self.write_count(instructions_bytes.len())?;
        self.write_all(&instructions_bytes)?;
        Ok(())
    }

    fn write_patch_instr(&mut self, instruction: &PatchInstr) -> io::Result<()> {
        let type_bits = match instruction {
            PatchInstr::Add => 0x00,
            PatchInstr::Sub => 0x01,
            PatchInstr::Mul => 0x02,
            PatchInstr::Div => 0x03,
            PatchInstr::Mod => 0x04,
            PatchInstr::Neg => 0x05,
            PatchInstr::Exp => 0x06,
            PatchInstr::BitOr => 0x10,
            PatchInstr::BitAnd => 0x11,
            PatchInstr::BitXor => 0x12,
            PatchInstr::Cmp => 0x13,
            PatchInstr::And => 0x21,
            PatchInstr::Or => 0x22,
            PatchInstr::Not => 0x23,
            PatchInstr::Eq => 0x30,
            PatchInstr::Ne => 0x31,
            PatchInstr::Gt => 0x32,
            PatchInstr::Lt => 0x33,
            PatchInstr::Ge => 0x34,
            PatchInstr::Le => 0x35,
            PatchInstr::ShiftLeft => 0x40,
            PatchInstr::SignedShiftRight => 0x41,
            PatchInstr::UnsignedShiftRight => 0x42,
            PatchInstr::SymbolBank(_) => 0x50,
            PatchInstr::SectionBank(_) => 0x51,
            PatchInstr::PcBank => 0x52,
            PatchInstr::SizeOfSection(_) => 0x53,
            PatchInstr::StartOfSection(_) => 0x54,
            PatchInstr::SizeOfType(_) => 0x55,
            PatchInstr::StartOfType(_) => 0x56,
            PatchInstr::LdhCheck => 0x60,
            PatchInstr::RstCheck => 0x61,
            PatchInstr::BitCheck(_) => 0x62,
            PatchInstr::High => 0x70,
            PatchInstr::Low => 0x71,
            PatchInstr::BitWidth => 0x72,
            PatchInstr::TrailingZeros => 0x73,
            PatchInstr::Int(_) => 0x80,
            PatchInstr::SymbolValue(_) => 0x81,
        };
        self.write_byte(type_bits)?;
        match instruction {
            PatchInstr::Add
            | PatchInstr::Sub
            | PatchInstr::Mul
            | PatchInstr::Div
            | PatchInstr::Mod
            | PatchInstr::Neg
            | PatchInstr::Exp
            | PatchInstr::BitOr
            | PatchInstr::BitAnd
            | PatchInstr::BitXor
            | PatchInstr::Cmp
            | PatchInstr::And
            | PatchInstr::Or
            | PatchInstr::Not
            | PatchInstr::Eq
            | PatchInstr::Ne
            | PatchInstr::Gt
            | PatchInstr::Lt
            | PatchInstr::Ge
            | PatchInstr::Le
            | PatchInstr::ShiftLeft
            | PatchInstr::SignedShiftRight
            | PatchInstr::UnsignedShiftRight
            | PatchInstr::PcBank
            | PatchInstr::LdhCheck
            | PatchInstr::RstCheck
            | PatchInstr::High
            | PatchInstr::Low
            | PatchInstr::BitWidth
            | PatchInstr::TrailingZeros => {}
            PatchInstr::SymbolBank(symbol_id) | PatchInstr::SymbolValue(symbol_id) => {
                self.write_long(symbol_id.0)?;
            }
            PatchInstr::SectionBank(section)
            | PatchInstr::SizeOfSection(section)
            | PatchInstr::StartOfSection(section) => {
                self.write_string(section)?;
            }
            PatchInstr::SizeOfType(kind) | PatchInstr::StartOfType(kind) => {
                self.write_byte(match kind {
                    SectionKind::WRam0 => 0,
                    SectionKind::VRam => 1,
                    SectionKind::RomX => 2,
                    SectionKind::Rom0 => 3,
                    SectionKind::HRam => 4,
                    SectionKind::WRamX => 5,
                    SectionKind::SRam => 6,
                    SectionKind::Oam => 7,
                })?;
            }
            PatchInstr::BitCheck(mask) => {
                self.write_byte(*mask)?;
            }
            PatchInstr::Int(int) => {
                self.write_long(*int)?;
            }
        }
        Ok(())
    }

    fn write_assertion(&mut self, assertion: &Assertion) -> io::Result<()> {
        self.write_long(assertion.node.0)?;
        self.write_long(assertion.line_no)?;
        self.write_long(assertion.offset)?;
        self.write_long(assertion.pc_section_id.0)?;
        self.write_long(assertion.pc_offset)?;
        let type_bits = match assertion.kind {
            AssertionKind::Warning => 0,
            AssertionKind::Error => 1,
            AssertionKind::Fatal => 2,
        };
        self.write_byte(type_bits)?;
        // we have to buffer writes for instructions first as the size is serialized
        // as the number of bytes, not the number of instructions
        let mut instructions_bytes = Vec::<u8>::new();
        for instruction in &assertion.instructions {
            instructions_bytes.write_patch_instr(instruction)?;
        }
        self.write_count(instructions_bytes.len())?;
        self.write_all(&instructions_bytes)?;
        self.write_string(&assertion.message)?;
        Ok(())
    }
}

impl<W: Write> ObjFileWriter for W {}

pub fn write(mut write: impl Write, obj_file: &ObjFile) -> io::Result<()> {
    write.write_obj_file(obj_file)
}
