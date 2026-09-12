use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::reg::Color;
use crate::{lir, opcode};
use indexmap::{IndexMap, IndexSet};
use quokka_lang_obj::{
    LinkingMode, ObjFile, PatchInstr, RamSectionData, RomSectionData, Section, SectionData,
    SectionId, SectionPatch, SectionPatchKind, SourceNode, SourceNodeData, SourceNodeId, Symbol,
    SymbolData, SymbolDefinition, SymbolId,
};
use slotmap::SecondaryMap;
use slotmap::secondary::Entry;
use std::collections::VecDeque;

#[derive(Default)]
pub struct ObjFileEncoder {
    #[allow(unused)]
    features: PreviewFeatureSet,
    file: ObjFile,
    memory_section_to_symbol: SecondaryMap<lir::SectionId, SymbolId>,
    code_section_to_id: SecondaryMap<lir::SectionId, SectionId>,
    fn_addr_to_symbol: IndexMap<lir::FnAddr, SymbolId>,
}

impl ObjFileEncoder {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    #[must_use]
    pub fn encode(mut self, lir: &lir::Lir) -> ObjFile {
        let source_id = self.file.insert_source_node(SourceNode {
            data: SourceNodeData::File {
                name: String::from("<input>"), // fix me
            },
            parent: None,
            parent_line_no: 0,
            quieted: false,
        });
        self.encode_lir(source_id, lir);
        self.file
    }

    fn encode_lir(&mut self, node: SourceNodeId, lir: &lir::Lir) {
        for (section_id, section) in &lir.sections {
            let symbol = match &section.data {
                lir::SectionData::Data(data) => self.encode_data_section(node, data),
                lir::SectionData::Ram(data) => self.encode_ram_section(node, data),
                _ => continue,
            };
            self.memory_section_to_symbol.insert(section_id, symbol);
        }
        for (section_id, section) in &lir.sections {
            let lir::SectionData::Code(code) = &section.data else {
                continue;
            };
            self.reserve_code_section(node, section_id, code);
        }
        for (section_id, section) in &lir.sections {
            let lir::SectionData::Code(code) = &section.data else {
                continue;
            };
            self.encode_code_section(node, section_id, code);
        }
    }

    fn encode_data_section(&mut self, node: SourceNodeId, data: &lir::DataSectionData) -> SymbolId {
        let rom_data = RomSectionData {
            bytes: data.data.clone(),
            patches: Vec::new(),
        };
        let (rom_data, bank) = match data.region.unwrap_or(lir::RomRegionKind::Rom0) {
            lir::RomRegionKind::Rom0 => (SectionData::Rom0(rom_data), None),
            lir::RomRegionKind::RomX(bank) => (
                SectionData::RomX(rom_data),
                bank.map(u16::from).map(u32::from),
            ),
        };
        let section = self.file.insert_section(Section {
            name: format!("data section {}", data.name),
            node,
            line_no: 0, // fix me
            data: rom_data,
            mode: LinkingMode::Normal,
            address: data.addr.map(u32::from),
            bank,
            alignment: 0,
            alignment_offset: 0,
        });
        let offset = 0;
        self.file.insert_symbol(Symbol {
            name: data.name.to_string(),
            data: SymbolData::Exported(SymbolDefinition {
                node,
                line_no: 0, // fix me
                section: Some(section),
                value: offset,
            }),
        })
    }

    fn encode_ram_section(&mut self, node: SourceNodeId, data: &lir::RamSectionData) -> SymbolId {
        let ram_data = RamSectionData {
            size: u32::from(data.size.bytes()),
        };
        let (ram_data, bank, mode) = match data.region {
            lir::RamRegionKind::WRam0 => (SectionData::WRam0(ram_data), None, LinkingMode::Normal),
            lir::RamRegionKind::VRam => (SectionData::VRam(ram_data), None, LinkingMode::Union),
            lir::RamRegionKind::HRam => (SectionData::HRam(ram_data), None, LinkingMode::Normal),
            lir::RamRegionKind::WRamX(bank) => (
                SectionData::WRam0(ram_data),
                bank.map(u16::from).map(u32::from),
                LinkingMode::Normal,
            ),
            lir::RamRegionKind::SRam => (SectionData::SRam(ram_data), None, LinkingMode::Normal),
            lir::RamRegionKind::Oam => (SectionData::Oam(ram_data), None, LinkingMode::Union),
        };
        let section = self.file.insert_section(Section {
            name: format!("ram section {}", data.name),
            node,
            line_no: 0, // fix me
            data: ram_data,
            mode,
            address: data.addr.map(u32::from),
            bank,
            alignment: 0,
            alignment_offset: 0,
        });
        let offset = 0;
        self.file.insert_symbol(Symbol {
            name: data.name.to_string(),
            data: SymbolData::Exported(SymbolDefinition {
                node,
                line_no: 0, // fix me
                section: Some(section),
                value: offset,
            }),
        })
    }

    fn reserve_code_section(
        &mut self,
        node: SourceNodeId,
        id: lir::SectionId,
        code: &lir::CodeSectionData,
    ) {
        let rom_data = RomSectionData {
            bytes: Vec::new(),
            patches: Vec::new(),
        };
        let (rom_data, bank) = match code.region.unwrap_or(lir::RomRegionKind::Rom0) {
            lir::RomRegionKind::Rom0 => (SectionData::Rom0(rom_data), None),
            lir::RomRegionKind::RomX(bank) => (
                SectionData::RomX(rom_data),
                bank.map(u16::from).map(u32::from),
            ),
        };
        let current_section_id = self.file.insert_section(Section {
            name: format!("code section {id:?}"),
            node,
            line_no: 0, // fix me
            data: rom_data,
            mode: LinkingMode::Normal,
            address: code.addr.map(u32::from),
            bank,
            alignment: 0,
            alignment_offset: 0,
        });
        self.code_section_to_id.insert(id, current_section_id);
        let mut block_to_function = SecondaryMap::<lir::BlockId, lir::BlockId>::new();
        for entry in code.functions.keys() {
            block_to_function.insert(entry, entry);
        }
        for function in code.functions.keys() {
            let mut queue = VecDeque::new();
            queue.push_back(function);
            while let Some(block) = queue.pop_front() {
                for next_block in code.blocks[block].next_blocks() {
                    match block_to_function.entry(next_block).unwrap() {
                        Entry::Occupied(_) => continue,
                        Entry::Vacant(entry) => {
                            entry.insert(function);
                        }
                    }
                    queue.push_back(next_block);
                }
            }
        }

        for block in code.blocks.keys() {
            let function = block_to_function.get(block).copied();
            let (name, exported) = match function {
                Some(function) if function == block => (code.functions[function].to_string(), true),
                Some(function) => (format!("{}{block}", &code.functions[function]), false),
                None => (format!("@DANGLING{block}"), false),
            };
            let definition = SymbolDefinition {
                node,
                line_no: 0, // fix me
                section: Some(current_section_id),
                value: 0,
            };
            let data = if exported {
                SymbolData::Exported(definition)
            } else {
                SymbolData::Local(definition)
            };
            let symbol_id = self.file.insert_symbol(Symbol { name, data });
            self.fn_addr_to_symbol.insert(
                lir::FnAddr {
                    section: Some(id),
                    entry_block: block,
                },
                symbol_id,
            );
        }
    }

    fn encode_code_section(
        &mut self,
        node: SourceNodeId,
        id: lir::SectionId,
        code: &lir::CodeSectionData,
    ) {
        let current_section_id = self.code_section_to_id[id];
        let mut bytes = Vec::new();
        let mut patches = Vec::new();
        let mut blocks = calculate_block_order(code).into_iter().peekable();
        while let Some(block_id) = blocks.next() {
            let symbol_id = self.fn_addr_to_symbol[&lir::FnAddr {
                section: Some(id),
                entry_block: block_id,
            }];
            self.file.symbols[symbol_id.index()]
                .data
                .as_definition_mut()
                .unwrap()
                .value = u32::try_from(bytes.len()).expect("u32 overflowed");
            let block = &code.blocks[block_id];
            for instruction in &block.instructions {
                self.encode_instruction(node, code, id, instruction, &mut bytes, &mut patches);
            }
            self.encode_terminator(
                node,
                code,
                id,
                &block.terminator,
                blocks.peek().copied(),
                &mut bytes,
                &mut patches,
            );
        }
        let data = self.file.sections[current_section_id.index()]
            .data
            .as_rom_data_mut()
            .unwrap();
        data.bytes = bytes;
        data.patches = patches;
    }

    fn encode_instruction(
        &self,
        node: SourceNodeId,
        code: &lir::CodeSectionData,
        id: lir::SectionId,
        instruction: &lir::Instr,
        bytes: &mut Vec<u8>,
        patches: &mut Vec<SectionPatch>,
    ) {
        match instruction {
            lir::Instr::Nop => bytes.push(opcode::NOP),
            lir::Instr::Ld8 { target, source } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::LD_B_B,
                    (Color::B, Color::C) => opcode::LD_B_C,
                    (Color::B, Color::D) => opcode::LD_B_D,
                    (Color::B, Color::E) => opcode::LD_B_E,
                    (Color::B, Color::H) => opcode::LD_B_H,
                    (Color::B, Color::L) => opcode::LD_B_L,
                    (Color::B, Color::A) => opcode::LD_B_A,
                    (Color::C, Color::B) => opcode::LD_C_B,
                    (Color::C, Color::C) => opcode::LD_C_C,
                    (Color::C, Color::D) => opcode::LD_C_D,
                    (Color::C, Color::E) => opcode::LD_C_E,
                    (Color::C, Color::H) => opcode::LD_C_H,
                    (Color::C, Color::L) => opcode::LD_C_L,
                    (Color::C, Color::A) => opcode::LD_C_A,
                    (Color::D, Color::B) => opcode::LD_D_B,
                    (Color::D, Color::C) => opcode::LD_D_C,
                    (Color::D, Color::D) => opcode::LD_D_D,
                    (Color::D, Color::E) => opcode::LD_D_E,
                    (Color::D, Color::H) => opcode::LD_D_H,
                    (Color::D, Color::L) => opcode::LD_D_L,
                    (Color::D, Color::A) => opcode::LD_D_A,
                    (Color::E, Color::B) => opcode::LD_E_B,
                    (Color::E, Color::C) => opcode::LD_E_C,
                    (Color::E, Color::D) => opcode::LD_E_D,
                    (Color::E, Color::E) => opcode::LD_E_E,
                    (Color::E, Color::H) => opcode::LD_E_H,
                    (Color::E, Color::L) => opcode::LD_E_L,
                    (Color::E, Color::A) => opcode::LD_E_A,
                    (Color::H, Color::B) => opcode::LD_H_B,
                    (Color::H, Color::C) => opcode::LD_H_C,
                    (Color::H, Color::D) => opcode::LD_H_D,
                    (Color::H, Color::E) => opcode::LD_H_E,
                    (Color::H, Color::H) => opcode::LD_H_H,
                    (Color::H, Color::L) => opcode::LD_H_L,
                    (Color::H, Color::A) => opcode::LD_H_A,
                    (Color::L, Color::B) => opcode::LD_L_B,
                    (Color::L, Color::C) => opcode::LD_L_C,
                    (Color::L, Color::D) => opcode::LD_L_D,
                    (Color::L, Color::E) => opcode::LD_L_E,
                    (Color::L, Color::H) => opcode::LD_L_H,
                    (Color::L, Color::L) => opcode::LD_L_L,
                    (Color::L, Color::A) => opcode::LD_L_A,
                    (Color::A, Color::B) => opcode::LD_A_B,
                    (Color::A, Color::C) => opcode::LD_A_C,
                    (Color::A, Color::D) => opcode::LD_A_D,
                    (Color::A, Color::E) => opcode::LD_A_E,
                    (Color::A, Color::H) => opcode::LD_A_H,
                    (Color::A, Color::L) => opcode::LD_A_L,
                    (Color::A, Color::A) => opcode::LD_A_A,
                    unexpected => unimplemented!("LD {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::Ld8Value { target, source } => {
                let opcode = match code.r8_registers[*target].color.unwrap() {
                    Color::B => opcode::LD_B_N8,
                    Color::C => opcode::LD_C_N8,
                    Color::D => opcode::LD_D_N8,
                    Color::E => opcode::LD_E_N8,
                    Color::H => opcode::LD_H_N8,
                    Color::L => opcode::LD_L_N8,
                    Color::A => opcode::LD_A_N8,
                    unexpected => unimplemented!("LD {unexpected:?}, {source}"),
                };
                bytes.push(opcode);
                bytes.push(*source);
            }
            lir::Instr::Ld8Ptr {
                target,
                source_addr,
            } => {
                let source_color = source_addr
                    .into_r16()
                    .map(|r16| code.r16_registers[r16].color.unwrap());
                let opcode = match (code.r8_registers[*target].color.unwrap(), source_color) {
                    (Color::B, Some(Color::HL)) => opcode::LD_B_HL,
                    (Color::C, Some(Color::HL)) => opcode::LD_C_HL,
                    (Color::D, Some(Color::HL)) => opcode::LD_D_HL,
                    (Color::E, Some(Color::HL)) => opcode::LD_E_HL,
                    (Color::H, Some(Color::HL)) => opcode::LD_H_HL,
                    (Color::L, Some(Color::HL)) => opcode::LD_L_HL,
                    (Color::A, Some(Color::HL)) => opcode::LD_A_HL,
                    (Color::A, Some(Color::BC)) => opcode::LD_A_BC,
                    (Color::A, Some(Color::DE)) => opcode::LD_A_DE,
                    (Color::A, None) => opcode::LD_A_A16,
                    (unexpected_target, Some(unexpected_source)) => {
                        unimplemented!("LD {unexpected_target:?}, [{unexpected_source:?}]")
                    }
                    (unexpected_target, None) => unimplemented!("LD {unexpected_target:?}, [r16]"),
                };
                self.encode_addr(node, id, opcode, source_addr, bytes, patches);
            }
            lir::Instr::Ld8HighPtr { source_addr, .. } => {
                let opcode = match source_addr
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::C) => opcode::LDH_A_C,
                    None => opcode::LDH_A_A8,
                    Some(unexpected) => unimplemented!("LDH A, [{unexpected:?}]"),
                };
                bytes.push(opcode);
                if let lir::HighAddr::Literal(addr) = source_addr {
                    bytes.push(*addr);
                }
            }
            lir::Instr::Ld8Inc { .. } => {
                bytes.push(opcode::LD_A_HLI);
            }
            lir::Instr::Ld8Dec { .. } => {
                bytes.push(opcode::LD_A_HLD);
            }
            lir::Instr::Ld16 { target, source } => {
                let opcode = match code.r16_registers[*target].color.unwrap() {
                    Color::BC => opcode::LD16_BC_N16,
                    Color::DE => opcode::LD16_DE_N16,
                    Color::HL => opcode::LD16_HL_N16,
                    unexpected => unimplemented!("LD {unexpected:?}, n16"),
                };
                self.encode_ld16_source(node, id, opcode, source, bytes, patches);
            }
            lir::Instr::LdSPValue { source } => {
                self.encode_ld16_source(node, id, opcode::LD16_SP_N16, source, bytes, patches);
            }
            lir::Instr::Store {
                target_addr,
                source,
            } => {
                let target_color = target_addr
                    .into_r16()
                    .map(|r16| code.r16_registers[r16].color.unwrap());
                let opcode = match (target_color, code.r8_registers[*source].color.unwrap()) {
                    (Some(Color::HL), Color::B) => opcode::LD_HL_B,
                    (Some(Color::HL), Color::C) => opcode::LD_HL_C,
                    (Some(Color::HL), Color::D) => opcode::LD_HL_D,
                    (Some(Color::HL), Color::E) => opcode::LD_HL_E,
                    (Some(Color::HL), Color::H) => opcode::LD_HL_H,
                    (Some(Color::HL), Color::L) => opcode::LD_HL_L,
                    (Some(Color::HL), Color::A) => opcode::LD_HL_A,
                    (Some(Color::BC), Color::A) => opcode::LD_BC_A,
                    (Some(Color::DE), Color::A) => opcode::LD_DE_A,
                    (None, Color::A) => opcode::LD_A_A16,
                    (Some(unexpected_target), unexpected_source) => {
                        unimplemented!("LD [{unexpected_target:?}], {unexpected_source:?}")
                    }
                    (None, unexpected_source) => unimplemented!("LD [r16], {unexpected_source:?}"),
                };
                self.encode_addr(node, id, opcode, target_addr, bytes, patches);
            }
            lir::Instr::StoreHigh { addr, .. } => {
                let opcode = match addr
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::C) => opcode::LDH_C_A,
                    None => opcode::LDH_A8_A,
                    Some(unexpected) => unimplemented!("LDH A, [{unexpected:?}]"),
                };
                bytes.push(opcode);
                if let lir::HighAddr::Literal(addr) = addr {
                    bytes.push(*addr);
                }
            }
            lir::Instr::StoreValue { source, .. } => {
                bytes.push(opcode::LD_HL_N8);
                bytes.push(*source);
            }
            lir::Instr::StoreInc { .. } => {
                bytes.push(opcode::LD_HLI_A);
            }
            lir::Instr::StoreDec { .. } => {
                bytes.push(opcode::LD_HLI_A);
            }
            lir::Instr::Adc { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::ADC_A_B,
                    Some(Color::C) => opcode::ADC_A_C,
                    Some(Color::D) => opcode::ADC_A_D,
                    Some(Color::E) => opcode::ADC_A_E,
                    Some(Color::H) => opcode::ADC_A_H,
                    Some(Color::L) => opcode::ADC_A_L,
                    None => opcode::ADC_A_HL,
                    Some(unexpected) => unimplemented!("ABC A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Add { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::ADD_A_B,
                    Some(Color::C) => opcode::ADD_A_C,
                    Some(Color::D) => opcode::ADD_A_D,
                    Some(Color::E) => opcode::ADD_A_E,
                    Some(Color::H) => opcode::ADD_A_H,
                    Some(Color::L) => opcode::ADD_A_L,
                    None => opcode::ADD_A_HL,
                    Some(unexpected) => unimplemented!("ADD A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Cp { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::CP_A_B,
                    Some(Color::C) => opcode::CP_A_C,
                    Some(Color::D) => opcode::CP_A_D,
                    Some(Color::E) => opcode::CP_A_E,
                    Some(Color::H) => opcode::CP_A_H,
                    Some(Color::L) => opcode::CP_A_L,
                    None => opcode::CP_A_HL,
                    Some(unexpected) => unimplemented!("CP A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Dec { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::DEC_B,
                    (Color::C, Color::C) => opcode::DEC_C,
                    (Color::D, Color::D) => opcode::DEC_D,
                    (Color::E, Color::E) => opcode::DEC_E,
                    (Color::H, Color::H) => opcode::DEC_H,
                    (Color::L, Color::L) => opcode::DEC_L,
                    unexpected => unimplemented!("DEC {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::DecPtr { .. } => {
                bytes.push(opcode::DEC_HL);
            }
            lir::Instr::Inc { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::INC_B,
                    (Color::C, Color::C) => opcode::INC_C,
                    (Color::D, Color::D) => opcode::INC_D,
                    (Color::E, Color::E) => opcode::INC_E,
                    (Color::H, Color::H) => opcode::INC_H,
                    (Color::L, Color::L) => opcode::INC_L,
                    unexpected => unimplemented!("INC {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::IncPtr { .. } => {
                bytes.push(opcode::INC_HL);
            }
            lir::Instr::Sbc { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::SBC_A_B,
                    Some(Color::C) => opcode::SBC_A_C,
                    Some(Color::D) => opcode::SBC_A_D,
                    Some(Color::E) => opcode::SBC_A_E,
                    Some(Color::H) => opcode::SBC_A_H,
                    Some(Color::L) => opcode::SBC_A_L,
                    None => opcode::SBC_A_HL,
                    Some(unexpected) => unimplemented!("SBC A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Sub { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::SUB_A_B,
                    Some(Color::C) => opcode::SUB_A_C,
                    Some(Color::D) => opcode::SUB_A_D,
                    Some(Color::E) => opcode::SUB_A_E,
                    Some(Color::H) => opcode::SUB_A_H,
                    Some(Color::L) => opcode::SUB_A_L,
                    None => opcode::SUB_A_HL,
                    Some(unexpected) => unimplemented!("SUB A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::And { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::AND_A_B,
                    Some(Color::C) => opcode::AND_A_C,
                    Some(Color::D) => opcode::AND_A_D,
                    Some(Color::E) => opcode::AND_A_E,
                    Some(Color::H) => opcode::AND_A_H,
                    Some(Color::L) => opcode::AND_A_L,
                    None => opcode::AND_A_HL,
                    Some(unexpected) => unimplemented!("AND A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Cpl { .. } => {
                bytes.push(opcode::CPL);
            }
            lir::Instr::Or { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::OR_A_B,
                    Some(Color::C) => opcode::OR_A_C,
                    Some(Color::D) => opcode::OR_A_D,
                    Some(Color::E) => opcode::OR_A_E,
                    Some(Color::H) => opcode::OR_A_H,
                    Some(Color::L) => opcode::OR_A_L,
                    None => opcode::OR_A_HL,
                    Some(unexpected) => unimplemented!("OR A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Xor { right, .. } => {
                let opcode = match right
                    .into_r8()
                    .map(|r8| code.r8_registers[r8].color.unwrap())
                {
                    Some(Color::B) => opcode::XOR_A_B,
                    Some(Color::C) => opcode::XOR_A_C,
                    Some(Color::D) => opcode::XOR_A_D,
                    Some(Color::E) => opcode::XOR_A_E,
                    Some(Color::H) => opcode::XOR_A_H,
                    Some(Color::L) => opcode::XOR_A_L,
                    None => opcode::XOR_A_HL,
                    Some(unexpected) => unimplemented!("XOR A, {unexpected:?}"),
                };
                bytes.push(opcode);
                if let lir::Operand8::Literal(value) = right {
                    bytes.push(*value);
                }
            }
            lir::Instr::Bit { bit, source, .. } => {
                let opcode = match (u8::from(*bit), code.r8_registers[*source].color.unwrap()) {
                    (0, Color::B) => opcode::BIT_0_B,
                    (0, Color::C) => opcode::BIT_0_C,
                    (0, Color::D) => opcode::BIT_0_D,
                    (0, Color::E) => opcode::BIT_0_E,
                    (0, Color::H) => opcode::BIT_0_H,
                    (0, Color::L) => opcode::BIT_0_L,
                    (0, Color::A) => opcode::BIT_0_A,
                    (1, Color::B) => opcode::BIT_1_B,
                    (1, Color::C) => opcode::BIT_1_C,
                    (1, Color::D) => opcode::BIT_1_D,
                    (1, Color::E) => opcode::BIT_1_E,
                    (1, Color::H) => opcode::BIT_1_H,
                    (1, Color::L) => opcode::BIT_1_L,
                    (1, Color::A) => opcode::BIT_1_A,
                    (2, Color::B) => opcode::BIT_2_B,
                    (2, Color::C) => opcode::BIT_2_C,
                    (2, Color::D) => opcode::BIT_2_D,
                    (2, Color::E) => opcode::BIT_2_E,
                    (2, Color::H) => opcode::BIT_2_H,
                    (2, Color::L) => opcode::BIT_2_L,
                    (2, Color::A) => opcode::BIT_2_A,
                    (3, Color::B) => opcode::BIT_3_B,
                    (3, Color::C) => opcode::BIT_3_C,
                    (3, Color::D) => opcode::BIT_3_D,
                    (3, Color::E) => opcode::BIT_3_E,
                    (3, Color::H) => opcode::BIT_3_H,
                    (3, Color::L) => opcode::BIT_3_L,
                    (3, Color::A) => opcode::BIT_3_A,
                    (4, Color::B) => opcode::BIT_4_B,
                    (4, Color::C) => opcode::BIT_4_C,
                    (4, Color::D) => opcode::BIT_4_D,
                    (4, Color::E) => opcode::BIT_4_E,
                    (4, Color::H) => opcode::BIT_4_H,
                    (4, Color::L) => opcode::BIT_4_L,
                    (4, Color::A) => opcode::BIT_4_A,
                    (5, Color::B) => opcode::BIT_5_B,
                    (5, Color::C) => opcode::BIT_5_C,
                    (5, Color::D) => opcode::BIT_5_D,
                    (5, Color::E) => opcode::BIT_5_E,
                    (5, Color::H) => opcode::BIT_5_H,
                    (5, Color::L) => opcode::BIT_5_L,
                    (5, Color::A) => opcode::BIT_5_A,
                    (6, Color::B) => opcode::BIT_6_B,
                    (6, Color::C) => opcode::BIT_6_C,
                    (6, Color::D) => opcode::BIT_6_D,
                    (6, Color::E) => opcode::BIT_6_E,
                    (6, Color::H) => opcode::BIT_6_H,
                    (6, Color::L) => opcode::BIT_6_L,
                    (6, Color::A) => opcode::BIT_6_A,
                    (7, Color::B) => opcode::BIT_7_B,
                    (7, Color::C) => opcode::BIT_7_C,
                    (7, Color::D) => opcode::BIT_7_D,
                    (7, Color::E) => opcode::BIT_7_E,
                    (7, Color::H) => opcode::BIT_7_H,
                    (7, Color::L) => opcode::BIT_7_L,
                    (7, Color::A) => opcode::BIT_7_A,
                    (unexpected_bit, unexpected_source) => {
                        unimplemented!("BIT {unexpected_bit}, {unexpected_source:?}")
                    }
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::BitPtr { bit, .. } => {
                let opcode = match u8::from(*bit) {
                    0 => opcode::BIT_0_HL,
                    1 => opcode::BIT_1_HL,
                    2 => opcode::BIT_2_HL,
                    3 => opcode::BIT_3_HL,
                    4 => opcode::BIT_4_HL,
                    5 => opcode::BIT_5_HL,
                    6 => opcode::BIT_6_HL,
                    7 => opcode::BIT_7_HL,
                    unexpected => unimplemented!("BIT {unexpected}, [HL]"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::Res {
                target,
                bit,
                source,
            } => {
                let opcode = match (
                    u8::from(*bit),
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (0, Color::B, Color::B) => opcode::RES_0_B,
                    (0, Color::C, Color::C) => opcode::RES_0_C,
                    (0, Color::D, Color::D) => opcode::RES_0_D,
                    (0, Color::E, Color::E) => opcode::RES_0_E,
                    (0, Color::H, Color::H) => opcode::RES_0_H,
                    (0, Color::L, Color::L) => opcode::RES_0_L,
                    (0, Color::A, Color::A) => opcode::RES_0_A,
                    (1, Color::B, Color::B) => opcode::RES_1_B,
                    (1, Color::C, Color::C) => opcode::RES_1_C,
                    (1, Color::D, Color::D) => opcode::RES_1_D,
                    (1, Color::E, Color::E) => opcode::RES_1_E,
                    (1, Color::H, Color::H) => opcode::RES_1_H,
                    (1, Color::L, Color::L) => opcode::RES_1_L,
                    (1, Color::A, Color::A) => opcode::RES_1_A,
                    (2, Color::B, Color::B) => opcode::RES_2_B,
                    (2, Color::C, Color::C) => opcode::RES_2_C,
                    (2, Color::D, Color::D) => opcode::RES_2_D,
                    (2, Color::E, Color::E) => opcode::RES_2_E,
                    (2, Color::H, Color::H) => opcode::RES_2_H,
                    (2, Color::L, Color::L) => opcode::RES_2_L,
                    (2, Color::A, Color::A) => opcode::RES_2_A,
                    (3, Color::B, Color::B) => opcode::RES_3_B,
                    (3, Color::C, Color::C) => opcode::RES_3_C,
                    (3, Color::D, Color::D) => opcode::RES_3_D,
                    (3, Color::E, Color::E) => opcode::RES_3_E,
                    (3, Color::H, Color::H) => opcode::RES_3_H,
                    (3, Color::L, Color::L) => opcode::RES_3_L,
                    (3, Color::A, Color::A) => opcode::RES_3_A,
                    (4, Color::B, Color::B) => opcode::RES_4_B,
                    (4, Color::C, Color::C) => opcode::RES_4_C,
                    (4, Color::D, Color::D) => opcode::RES_4_D,
                    (4, Color::E, Color::E) => opcode::RES_4_E,
                    (4, Color::H, Color::H) => opcode::RES_4_H,
                    (4, Color::L, Color::L) => opcode::RES_4_L,
                    (4, Color::A, Color::A) => opcode::RES_4_A,
                    (5, Color::B, Color::B) => opcode::RES_5_B,
                    (5, Color::C, Color::C) => opcode::RES_5_C,
                    (5, Color::D, Color::D) => opcode::RES_5_D,
                    (5, Color::E, Color::E) => opcode::RES_5_E,
                    (5, Color::H, Color::H) => opcode::RES_5_H,
                    (5, Color::L, Color::L) => opcode::RES_5_L,
                    (5, Color::A, Color::A) => opcode::RES_5_A,
                    (6, Color::B, Color::B) => opcode::RES_6_B,
                    (6, Color::C, Color::C) => opcode::RES_6_C,
                    (6, Color::D, Color::D) => opcode::RES_6_D,
                    (6, Color::E, Color::E) => opcode::RES_6_E,
                    (6, Color::H, Color::H) => opcode::RES_6_H,
                    (6, Color::L, Color::L) => opcode::RES_6_L,
                    (6, Color::A, Color::A) => opcode::RES_6_A,
                    (7, Color::B, Color::B) => opcode::RES_7_B,
                    (7, Color::C, Color::C) => opcode::RES_7_C,
                    (7, Color::D, Color::D) => opcode::RES_7_D,
                    (7, Color::E, Color::E) => opcode::RES_7_E,
                    (7, Color::H, Color::H) => opcode::RES_7_H,
                    (7, Color::L, Color::L) => opcode::RES_7_L,
                    (7, Color::A, Color::A) => opcode::RES_7_A,
                    (unexpected_bit, unexpected_target, unexpected_source) => unimplemented!(
                        "RES {unexpected_bit}, ({unexpected_target:?}, {unexpected_source:?})"
                    ),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::ResPtr { bit, .. } => {
                let opcode = match u8::from(*bit) {
                    0 => opcode::RES_0_HL,
                    1 => opcode::RES_1_HL,
                    2 => opcode::RES_2_HL,
                    3 => opcode::RES_3_HL,
                    4 => opcode::RES_4_HL,
                    5 => opcode::RES_5_HL,
                    6 => opcode::RES_6_HL,
                    7 => opcode::RES_7_HL,
                    unexpected => unimplemented!("RES {unexpected}, [HL]"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::Set {
                target,
                bit,
                source,
            } => {
                let opcode = match (
                    u8::from(*bit),
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (0, Color::B, Color::B) => opcode::SET_0_B,
                    (0, Color::C, Color::C) => opcode::SET_0_C,
                    (0, Color::D, Color::D) => opcode::SET_0_D,
                    (0, Color::E, Color::E) => opcode::SET_0_E,
                    (0, Color::H, Color::H) => opcode::SET_0_H,
                    (0, Color::L, Color::L) => opcode::SET_0_L,
                    (0, Color::A, Color::A) => opcode::SET_0_A,
                    (1, Color::B, Color::B) => opcode::SET_1_B,
                    (1, Color::C, Color::C) => opcode::SET_1_C,
                    (1, Color::D, Color::D) => opcode::SET_1_D,
                    (1, Color::E, Color::E) => opcode::SET_1_E,
                    (1, Color::H, Color::H) => opcode::SET_1_H,
                    (1, Color::L, Color::L) => opcode::SET_1_L,
                    (1, Color::A, Color::A) => opcode::SET_1_A,
                    (2, Color::B, Color::B) => opcode::SET_2_B,
                    (2, Color::C, Color::C) => opcode::SET_2_C,
                    (2, Color::D, Color::D) => opcode::SET_2_D,
                    (2, Color::E, Color::E) => opcode::SET_2_E,
                    (2, Color::H, Color::H) => opcode::SET_2_H,
                    (2, Color::L, Color::L) => opcode::SET_2_L,
                    (2, Color::A, Color::A) => opcode::SET_2_A,
                    (3, Color::B, Color::B) => opcode::SET_3_B,
                    (3, Color::C, Color::C) => opcode::SET_3_C,
                    (3, Color::D, Color::D) => opcode::SET_3_D,
                    (3, Color::E, Color::E) => opcode::SET_3_E,
                    (3, Color::H, Color::H) => opcode::SET_3_H,
                    (3, Color::L, Color::L) => opcode::SET_3_L,
                    (3, Color::A, Color::A) => opcode::SET_3_A,
                    (4, Color::B, Color::B) => opcode::SET_4_B,
                    (4, Color::C, Color::C) => opcode::SET_4_C,
                    (4, Color::D, Color::D) => opcode::SET_4_D,
                    (4, Color::E, Color::E) => opcode::SET_4_E,
                    (4, Color::H, Color::H) => opcode::SET_4_H,
                    (4, Color::L, Color::L) => opcode::SET_4_L,
                    (4, Color::A, Color::A) => opcode::SET_4_A,
                    (5, Color::B, Color::B) => opcode::SET_5_B,
                    (5, Color::C, Color::C) => opcode::SET_5_C,
                    (5, Color::D, Color::D) => opcode::SET_5_D,
                    (5, Color::E, Color::E) => opcode::SET_5_E,
                    (5, Color::H, Color::H) => opcode::SET_5_H,
                    (5, Color::L, Color::L) => opcode::SET_5_L,
                    (5, Color::A, Color::A) => opcode::SET_5_A,
                    (6, Color::B, Color::B) => opcode::SET_6_B,
                    (6, Color::C, Color::C) => opcode::SET_6_C,
                    (6, Color::D, Color::D) => opcode::SET_6_D,
                    (6, Color::E, Color::E) => opcode::SET_6_E,
                    (6, Color::H, Color::H) => opcode::SET_6_H,
                    (6, Color::L, Color::L) => opcode::SET_6_L,
                    (6, Color::A, Color::A) => opcode::SET_6_A,
                    (7, Color::B, Color::B) => opcode::SET_7_B,
                    (7, Color::C, Color::C) => opcode::SET_7_C,
                    (7, Color::D, Color::D) => opcode::SET_7_D,
                    (7, Color::E, Color::E) => opcode::SET_7_E,
                    (7, Color::H, Color::H) => opcode::SET_7_H,
                    (7, Color::L, Color::L) => opcode::SET_7_L,
                    (7, Color::A, Color::A) => opcode::SET_7_A,
                    (unexpected_bit, unexpected_target, unexpected_source) => unimplemented!(
                        "SET {unexpected_bit}, ({unexpected_target:?}, {unexpected_source:?})"
                    ),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::SetPtr { bit, .. } => {
                let opcode = match u8::from(*bit) {
                    0 => opcode::SET_0_HL,
                    1 => opcode::SET_1_HL,
                    2 => opcode::SET_2_HL,
                    3 => opcode::SET_3_HL,
                    4 => opcode::SET_4_HL,
                    5 => opcode::SET_5_HL,
                    6 => opcode::SET_6_HL,
                    7 => opcode::SET_7_HL,
                    unexpected => unimplemented!("SET {unexpected}, [HL]"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::Rl { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::RL_B,
                    (Color::C, Color::C) => opcode::RL_C,
                    (Color::D, Color::D) => opcode::RL_D,
                    (Color::E, Color::E) => opcode::RL_E,
                    (Color::H, Color::H) => opcode::RL_H,
                    (Color::L, Color::L) => opcode::RL_L,
                    unexpected => unimplemented!("RL {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::RlPtr { .. } => {
                bytes.extend(opcode::RL_HL.to_le_bytes());
            }
            lir::Instr::Rlc { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::RLC_B,
                    (Color::C, Color::C) => opcode::RLC_C,
                    (Color::D, Color::D) => opcode::RLC_D,
                    (Color::E, Color::E) => opcode::RLC_E,
                    (Color::H, Color::H) => opcode::RLC_H,
                    (Color::L, Color::L) => opcode::RLC_L,
                    unexpected => unimplemented!("RLC {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::RlcPtr { .. } => {
                bytes.extend(opcode::RLC_HL.to_le_bytes());
            }
            lir::Instr::Rr { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::RR_B,
                    (Color::C, Color::C) => opcode::RR_C,
                    (Color::D, Color::D) => opcode::RR_D,
                    (Color::E, Color::E) => opcode::RR_E,
                    (Color::H, Color::H) => opcode::RR_H,
                    (Color::L, Color::L) => opcode::RR_L,
                    unexpected => unimplemented!("RR {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::RrPtr { .. } => {
                bytes.extend(opcode::RR_HL.to_le_bytes());
            }
            lir::Instr::Rrc { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::RRC_B,
                    (Color::C, Color::C) => opcode::RRC_C,
                    (Color::D, Color::D) => opcode::RRC_D,
                    (Color::E, Color::E) => opcode::RRC_E,
                    (Color::H, Color::H) => opcode::RRC_H,
                    (Color::L, Color::L) => opcode::RRC_L,
                    unexpected => unimplemented!("RRC {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::RrcPtr { .. } => {
                bytes.extend(opcode::RRC_HL.to_le_bytes());
            }
            lir::Instr::Rla { .. } => {
                bytes.push(opcode::RLA);
            }
            lir::Instr::Rlca { .. } => {
                bytes.push(opcode::RLCA);
            }
            lir::Instr::Rra { .. } => {
                bytes.push(opcode::RRA);
            }
            lir::Instr::Rrca { .. } => {
                bytes.push(opcode::RRCA);
            }
            lir::Instr::Sla { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::SLA_B,
                    (Color::C, Color::C) => opcode::SLA_C,
                    (Color::D, Color::D) => opcode::SLA_D,
                    (Color::E, Color::E) => opcode::SLA_E,
                    (Color::H, Color::H) => opcode::SLA_H,
                    (Color::L, Color::L) => opcode::SLA_L,
                    unexpected => unimplemented!("SLA {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::SlaPtr { .. } => {
                bytes.extend(opcode::SLA_HL.to_le_bytes());
            }
            lir::Instr::Sra { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::SRA_B,
                    (Color::C, Color::C) => opcode::SRA_C,
                    (Color::D, Color::D) => opcode::SRA_D,
                    (Color::E, Color::E) => opcode::SRA_E,
                    (Color::H, Color::H) => opcode::SRA_H,
                    (Color::L, Color::L) => opcode::SRA_L,
                    unexpected => unimplemented!("SRA {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::SraPtr { .. } => {
                bytes.extend(opcode::SRA_HL.to_le_bytes());
            }
            lir::Instr::Srl { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::SRL_B,
                    (Color::C, Color::C) => opcode::SRL_C,
                    (Color::D, Color::D) => opcode::SRL_D,
                    (Color::E, Color::E) => opcode::SRL_E,
                    (Color::H, Color::H) => opcode::SRL_H,
                    (Color::L, Color::L) => opcode::SRL_L,
                    unexpected => unimplemented!("SRL {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::SrlPtr { .. } => {
                bytes.extend(opcode::SRL_HL.to_le_bytes());
            }
            lir::Instr::Swap { target, source, .. } => {
                let opcode = match (
                    code.r8_registers[*target].color.unwrap(),
                    code.r8_registers[*source].color.unwrap(),
                ) {
                    (Color::B, Color::B) => opcode::SWAP_B,
                    (Color::C, Color::C) => opcode::SWAP_C,
                    (Color::D, Color::D) => opcode::SWAP_D,
                    (Color::E, Color::E) => opcode::SWAP_E,
                    (Color::H, Color::H) => opcode::SWAP_H,
                    (Color::L, Color::L) => opcode::SWAP_L,
                    unexpected => unimplemented!("SWAP {unexpected:?}"),
                };
                bytes.extend(opcode.to_le_bytes());
            }
            lir::Instr::SwapPtr { .. } => {
                bytes.extend(opcode::SWAP_HL.to_le_bytes());
            }
            lir::Instr::Ccf { .. } => {
                bytes.push(opcode::CCF);
            }
            lir::Instr::Scf { .. } => {
                bytes.push(opcode::SCF);
            }
            lir::Instr::Add16 { right, .. } => {
                let opcode = match code.r16_registers[*right].color.unwrap() {
                    Color::BC => opcode::ADD_HL_BC,
                    Color::DE => opcode::ADD_HL_DE,
                    Color::HL => opcode::ADD_HL_HL,
                    unexpected => unimplemented!("ADD HL, {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::AddSP { right, .. } => {
                bytes.push(opcode::ADD_SP_E8);
                bytes.extend(right.to_le_bytes());
            }
            lir::Instr::LdSPE8 { right, .. } => {
                bytes.push(opcode::LD_HL_SP_E8);
                bytes.extend(right.to_le_bytes());
            }
            lir::Instr::Dec16 { target, source } => {
                let opcode = match (
                    code.r16_registers[*target].color.unwrap(),
                    code.r16_registers[*source].color.unwrap(),
                ) {
                    (Color::BC, Color::BC) => opcode::DEC16_BC,
                    (Color::DE, Color::DE) => opcode::DEC16_DE,
                    (Color::HL, Color::HL) => opcode::DEC16_HL,
                    unexpected => unimplemented!("DEC {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::Inc16 { target, source } => {
                let opcode = match (
                    code.r16_registers[*target].color.unwrap(),
                    code.r16_registers[*source].color.unwrap(),
                ) {
                    (Color::BC, Color::BC) => opcode::INC16_BC,
                    (Color::DE, Color::DE) => opcode::INC16_DE,
                    (Color::HL, Color::HL) => opcode::INC16_HL,
                    unexpected => unimplemented!("INC {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::PushR16 { source } => {
                let opcode = match code.r16_registers[*source].color.unwrap() {
                    Color::BC => opcode::PUSH_BC,
                    Color::DE => opcode::PUSH_DE,
                    Color::HL => opcode::PUSH_HL,
                    unexpected => unimplemented!("PUSH {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::PushAF { .. } => {
                bytes.push(opcode::PUSH_AF);
            }
            lir::Instr::PopR16 { target } => {
                let opcode = match code.r16_registers[*target].color.unwrap() {
                    Color::BC => opcode::POP_BC,
                    Color::DE => opcode::POP_DE,
                    Color::HL => opcode::POP_HL,
                    unexpected => unimplemented!("POP {unexpected:?}"),
                };
                bytes.push(opcode);
            }
            lir::Instr::PopAF { .. } => {
                bytes.push(opcode::POP_AF);
            }
            lir::Instr::Join { .. } => {}
            lir::Instr::Split { .. } => {}
            lir::Instr::MoveZero { .. } => {}
            lir::Instr::MoveCarry { .. } => {}
            lir::Instr::LdReturn8 { .. } => {}
            lir::Instr::LdReturn16 { .. } => {}
        }
    }

    fn encode_terminator(
        &self,
        node: SourceNodeId,
        code: &lir::CodeSectionData,
        id: lir::SectionId,
        terminator: &lir::Terminator,
        peeked_block_id: Option<lir::BlockId>,
        bytes: &mut Vec<u8>,
        patches: &mut Vec<SectionPatch>,
    ) {
        let current_section_id = self.code_section_to_id[id];
        let next_block = match terminator {
            lir::Terminator::Call {
                callee,
                return_block,
            } => {
                let callee = lir::FnAddr {
                    section: Some(callee.section.unwrap_or(id)),
                    entry_block: callee.entry_block,
                };
                let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.push(opcode::CALL_A16);
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![PatchInstr::SymbolValue(self.fn_addr_to_symbol[&callee])],
                });
                Some(*return_block)
            }
            lir::Terminator::CallIf {
                source,
                negate,
                callee,
                return_block,
                else_block,
            } => {
                assert_eq!(
                    return_block, else_block,
                    "todo, CALLIF should not have different return and else blocks"
                );

                let opcode = match (negate, source) {
                    (true, lir::FlagId::Zero(_)) => opcode::CALL_NZ_A16,
                    (false, lir::FlagId::Zero(_)) => opcode::CALL_Z_A16,
                    (true, lir::FlagId::Carry(_)) => opcode::CALL_NC_A16,
                    (false, lir::FlagId::Carry(_)) => opcode::CALL_C_A16,
                };

                let callee = lir::FnAddr {
                    section: Some(callee.section.unwrap_or(id)),
                    entry_block: callee.entry_block,
                };
                let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.push(opcode);
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![PatchInstr::SymbolValue(self.fn_addr_to_symbol[&callee])],
                });
                Some(*return_block)
            }
            lir::Terminator::CallR16 {
                callee,
                return_block,
            } => {
                todo!("CALL R16");
                Some(*return_block)
            }
            lir::Terminator::Jp { next_block } => Some(*next_block),
            lir::Terminator::JpFn { addr } => {
                let addr = lir::FnAddr {
                    section: Some(addr.section.unwrap_or(id)),
                    entry_block: addr.entry_block,
                };
                let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.push(opcode::JP_A16);
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![PatchInstr::SymbolValue(self.fn_addr_to_symbol[&addr])],
                });
                None
            }
            lir::Terminator::JpIf {
                source,
                negate,
                then_block,
                else_block,
            } => {
                let mut then_block = *then_block;
                let mut else_block = *else_block;
                let mut negate = *negate;
                // if the then block is going to be the next block written, we can flip the
                // condition and avoid an additional JP instruction
                if Some(then_block) == peeked_block_id {
                    std::mem::swap(&mut then_block, &mut else_block);
                    negate = !negate;
                }

                let opcode = match (negate, source) {
                    (true, lir::FlagId::Zero(_)) => opcode::JP_NZ_A16,
                    (false, lir::FlagId::Zero(_)) => opcode::JP_Z_A16,
                    (true, lir::FlagId::Carry(_)) => opcode::JP_NC_A16,
                    (false, lir::FlagId::Carry(_)) => opcode::JP_C_A16,
                };

                let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.push(opcode);
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![PatchInstr::SymbolValue(
                        self.fn_addr_to_symbol[&lir::FnAddr {
                            section: Some(id),
                            entry_block: then_block,
                        }],
                    )],
                });

                Some(else_block)
            }
            lir::Terminator::Ret => {
                bytes.push(opcode::RET);
                None
            }
            lir::Terminator::RetIf {
                source,
                negate,
                else_block,
            } => {
                let opcode = match (negate, source) {
                    (true, lir::FlagId::Zero(_)) => opcode::RET_NZ,
                    (false, lir::FlagId::Zero(_)) => opcode::RET_Z,
                    (true, lir::FlagId::Carry(_)) => opcode::RET_NC,
                    (false, lir::FlagId::Carry(_)) => opcode::RET_C,
                };
                bytes.push(opcode);
                Some(*else_block)
            }
            lir::Terminator::Halt { return_block } => Some(*return_block),
        };

        if let Some(next_block) = next_block
            && Some(next_block) != peeked_block_id
        {
            let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
            bytes.push(opcode::JP_A16);
            let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
            bytes.extend(0u16.to_le_bytes());
            patches.push(SectionPatch {
                node,
                line_no: 0,
                offset,
                pc_section_id: current_section_id,
                pc_offset,
                kind: SectionPatchKind::Byte2,
                instructions: vec![PatchInstr::SymbolValue(
                    self.fn_addr_to_symbol[&lir::FnAddr {
                        section: Some(id),
                        entry_block: next_block,
                    }],
                )],
            });
        }
    }

    fn encode_ld16_source(
        &self,
        node: SourceNodeId,
        id: lir::SectionId,
        opcode: u8,
        source: &lir::Ld16Source,
        bytes: &mut Vec<u8>,
        patches: &mut Vec<SectionPatch>,
    ) {
        let current_section_id = self.code_section_to_id[id];
        let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
        bytes.push(opcode);
        match source {
            lir::Ld16Source::Literal(source) => bytes.extend(source.to_le_bytes()),
            lir::Ld16Source::FnAddr(addr) => {
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                let addr = lir::FnAddr {
                    section: Some(addr.section.unwrap_or(id)),
                    entry_block: addr.entry_block,
                };
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![PatchInstr::SymbolValue(self.fn_addr_to_symbol[&addr])],
                });
            }
            lir::Ld16Source::SectionAddr(addr) => {
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![
                        PatchInstr::SymbolValue(self.memory_section_to_symbol[addr.section]),
                        PatchInstr::Int(u32::from(addr.offset)),
                        PatchInstr::Add,
                    ],
                });
            }
        }
    }

    fn encode_addr(
        &self,
        node: SourceNodeId,
        id: lir::SectionId,
        opcode: u8,
        addr: &lir::Addr,
        bytes: &mut Vec<u8>,
        patches: &mut Vec<SectionPatch>,
    ) {
        let current_section_id = self.code_section_to_id[id];
        let pc_offset = u32::try_from(bytes.len()).expect("u32 overflowed");
        bytes.push(opcode);
        match addr {
            lir::Addr::Reg(_) => {}
            lir::Addr::Literal(addr) => bytes.extend(addr.to_le_bytes()),
            lir::Addr::Section {
                section,
                offset: section_offset,
            } => {
                let offset = u32::try_from(bytes.len()).expect("u32 overflowed");
                bytes.extend(0u16.to_le_bytes());
                patches.push(SectionPatch {
                    node,
                    line_no: 0,
                    offset,
                    pc_section_id: current_section_id,
                    pc_offset,
                    kind: SectionPatchKind::Byte2,
                    instructions: vec![
                        PatchInstr::SymbolValue(self.memory_section_to_symbol[*section]),
                        PatchInstr::Int(u32::from(*section_offset)),
                        PatchInstr::Add,
                    ],
                });
            }
        }
    }
}

fn calculate_block_order(code: &lir::CodeSectionData) -> IndexSet<lir::BlockId> {
    let mut ordered_blocks = IndexSet::new();
    for function in code.functions.keys() {
        ordered_blocks.extend(reverse_postorder_blocks(code, function));
    }
    // handle dangling blocks
    for block in code.blocks.keys() {
        if !ordered_blocks.contains(&block) {
            ordered_blocks.extend(reverse_postorder_blocks(code, block));
        }
    }
    ordered_blocks
}

fn reverse_postorder_blocks(
    code: &lir::CodeSectionData,
    source: lir::BlockId,
) -> IndexSet<lir::BlockId> {
    let mut result = IndexSet::<lir::BlockId>::new();
    result.reverse();
    let mut visited: IndexSet<lir::BlockId> = IndexSet::new();
    let mut stack: Vec<(lir::BlockId, VecDeque<lir::BlockId>)> = vec![(
        source,
        code.blocks[source].next_blocks().collect::<VecDeque<_>>(),
    )];
    visited.insert(source);
    while let Some(&mut (node, ref mut edges)) = stack.last_mut() {
        match edges.pop_front() {
            Some(forward_node) => {
                if visited.insert(forward_node) {
                    stack.push((
                        forward_node,
                        code.blocks[forward_node]
                            .next_blocks()
                            .collect::<VecDeque<_>>(),
                    ));
                }
            }
            None => {
                stack.pop();
                result.insert(node);
            }
        }
    }
    result.reverse();
    result
}
