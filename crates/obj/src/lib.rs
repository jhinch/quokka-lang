//! Crate designed for reading, writing and manipulating object files compatible with RGBDS
//! Crate was written as a clean-room based off the [reference documentation](https://rgbds.gbdev.io/docs/v1.0.3/rgbds.5).

mod file;
mod read;
mod write;

pub use file::*;
pub use read::*;
pub use write::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::io::Cursor;

    fn round_trip(obj_file: &ObjFile) -> io::Result<ObjFile> {
        let mut bytes = Vec::new();
        write(&mut bytes, obj_file)?;
        read(Cursor::new(bytes))
    }

    fn assert_round_trips(obj_file: &ObjFile) {
        let result = round_trip(obj_file).expect("round trip should succeed");
        assert_eq!(&result, obj_file);
    }

    fn empty_obj_file() -> ObjFile {
        ObjFile {
            revision: 0,
            source_nodes: Vec::new(),
            symbols: Vec::new(),
            sections: Vec::new(),
        }
    }

    #[test]
    fn round_trips_empty_file() {
        assert_round_trips(&empty_obj_file());
    }

    #[test]
    fn round_trips_source_nodes() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![
            SourceNode {
                parent: None,
                parent_line_no: 0,
                data: SourceNodeData::File {
                    name: "main.asm".to_string(),
                },
                quieted: false,
            },
            SourceNode {
                parent: Some(SourceNodeId(0)),
                parent_line_no: 10,
                data: SourceNodeData::Macro {
                    name: "MY_MACRO".to_string(),
                },
                quieted: true,
            },
            SourceNode {
                parent: Some(SourceNodeId(1)),
                parent_line_no: 20,
                data: SourceNodeData::Repeat {
                    iterations: vec![1, 2, 3],
                },
                quieted: false,
            },
            SourceNode {
                parent: Some(SourceNodeId(1)),
                parent_line_no: 21,
                data: SourceNodeData::Repeat { iterations: vec![] },
                quieted: false,
            },
        ];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn round_trips_symbols() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![SourceNode {
            parent: None,
            parent_line_no: 0,
            data: SourceNodeData::File {
                name: "main.asm".to_string(),
            },
            quieted: false,
        }];
        obj_file.symbols = vec![
            Symbol {
                name: "ImportedSymbol".to_string(),
                data: SymbolData::Import,
            },
            Symbol {
                name: "LocalSymbol".to_string(),
                data: SymbolData::Local(SymbolDefinition {
                    node: SourceNodeId(0),
                    line_no: 5,
                    section: None,
                    value: 42,
                }),
            },
            Symbol {
                name: "ExportedSymbol".to_string(),
                data: SymbolData::Exported(SymbolDefinition {
                    node: SourceNodeId(0),
                    line_no: 6,
                    section: Some(SectionId(0)),
                    value: 0,
                }),
            },
        ];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn round_trips_ram_sections() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![SourceNode {
            parent: None,
            parent_line_no: 0,
            data: SourceNodeData::File {
                name: "main.asm".to_string(),
            },
            quieted: false,
        }];
        let ram_section = |name: &str, data: fn(RamSectionData) -> SectionData| Section {
            name: name.to_string(),
            node: SourceNodeId(0),
            line_no: 1,
            data: data(RamSectionData { size: 16 }),
            mode: LinkingMode::Normal,
            address: Some(0x8000),
            bank: Some(1),
            alignment: 3,
            alignment_offset: 7,
        };
        obj_file.sections = vec![
            ram_section("WRam0", SectionData::WRam0),
            ram_section("VRam", SectionData::VRam),
            ram_section("HRam", SectionData::HRam),
            ram_section("WRamX", SectionData::WRamX),
            ram_section("SRam", SectionData::SRam),
            ram_section("Oam", SectionData::Oam),
        ];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn round_trips_rom_sections_with_patches() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![SourceNode {
            parent: None,
            parent_line_no: 0,
            data: SourceNodeData::File {
                name: "main.asm".to_string(),
            },
            quieted: false,
        }];
        obj_file.symbols = vec![Symbol {
            name: "SomeSymbol".to_string(),
            data: SymbolData::Import,
        }];
        obj_file.sections = vec![
            Section {
                name: "ROM0 Section".to_string(),
                node: SourceNodeId(0),
                line_no: 1,
                data: SectionData::Rom0(RomSectionData {
                    bytes: vec![0x00, 0x01, 0x02, 0x03],
                    patches: vec![SectionPatch {
                        node: SourceNodeId(0),
                        line_no: 2,
                        offset: 1,
                        pc_section_id: SectionId(0),
                        pc_offset: 1,
                        kind: SectionPatchKind::Byte1,
                        instructions: all_patch_instructions(),
                    }],
                }),
                mode: LinkingMode::Normal,
                address: None,
                bank: None,
                alignment: 0,
                alignment_offset: 0,
            },
            Section {
                name: "ROMX Section".to_string(),
                node: SourceNodeId(0),
                line_no: 3,
                data: SectionData::RomX(RomSectionData {
                    bytes: vec![],
                    patches: vec![],
                }),
                mode: LinkingMode::Normal,
                address: None,
                bank: Some(2),
                alignment: 0,
                alignment_offset: 0,
            },
        ];
        assert_round_trips(&obj_file);
    }

    fn all_patch_instructions() -> Vec<PatchInstr> {
        vec![
            PatchInstr::Add,
            PatchInstr::Sub,
            PatchInstr::Mul,
            PatchInstr::Div,
            PatchInstr::Mod,
            PatchInstr::Neg,
            PatchInstr::Exp,
            PatchInstr::BitOr,
            PatchInstr::BitAnd,
            PatchInstr::BitXor,
            PatchInstr::Cmp,
            PatchInstr::And,
            PatchInstr::Or,
            PatchInstr::Not,
            PatchInstr::Eq,
            PatchInstr::Ne,
            PatchInstr::Gt,
            PatchInstr::Lt,
            PatchInstr::Ge,
            PatchInstr::Le,
            PatchInstr::ShiftLeft,
            PatchInstr::SignedShiftRight,
            PatchInstr::UnsignedShiftRight,
            PatchInstr::SymbolBank(SymbolId(0)),
            PatchInstr::SectionBank("OtherSection".to_string()),
            PatchInstr::PcBank,
            PatchInstr::SizeOfSection("OtherSection".to_string()),
            PatchInstr::StartOfSection("OtherSection".to_string()),
            PatchInstr::SizeOfType(SectionKind::RomX),
            PatchInstr::StartOfType(SectionKind::WRam0),
            PatchInstr::LdhCheck,
            PatchInstr::RstCheck,
            PatchInstr::BitCheck(5),
            PatchInstr::High,
            PatchInstr::Low,
            PatchInstr::BitWidth,
            PatchInstr::TrailingZeros,
            PatchInstr::Int(0x1234_5678),
            PatchInstr::SymbolValue(SymbolId(0)),
        ]
    }

    #[test]
    fn round_trips_each_section_kind_for_size_of_and_start_of() {
        let kinds = [
            SectionKind::WRam0,
            SectionKind::VRam,
            SectionKind::RomX,
            SectionKind::Rom0,
            SectionKind::HRam,
            SectionKind::WRamX,
            SectionKind::SRam,
            SectionKind::Oam,
        ];
        for kind in kinds {
            let mut obj_file = empty_obj_file();
            obj_file.sections = vec![Section {
                name: "Section".to_string(),
                node: SourceNodeId(0),
                line_no: 0,
                data: SectionData::Rom0(RomSectionData {
                    bytes: vec![],
                    patches: vec![SectionPatch {
                        node: SourceNodeId(0),
                        line_no: 0,
                        offset: 0,
                        pc_section_id: SectionId(0),
                        pc_offset: 0,
                        kind: SectionPatchKind::Byte1,
                        instructions: vec![
                            PatchInstr::SizeOfType(kind),
                            PatchInstr::StartOfType(kind),
                        ],
                    }],
                }),
                mode: LinkingMode::Normal,
                address: None,
                bank: None,
                alignment: 0,
                alignment_offset: 0,
            }];
            assert_round_trips(&obj_file);
        }
    }

    #[test]
    fn round_trips_each_linking_mode() {
        for mode in [
            LinkingMode::Normal,
            LinkingMode::Union,
            LinkingMode::Fragment,
        ] {
            let mut obj_file = empty_obj_file();
            obj_file.sections = vec![Section {
                name: "Section".to_string(),
                node: SourceNodeId(0),
                line_no: 0,
                data: SectionData::WRam0(RamSectionData { size: 4 }),
                mode,
                address: None,
                bank: None,
                alignment: 0,
                alignment_offset: 0,
            }];
            assert_round_trips(&obj_file);
        }
    }

    #[test]
    fn round_trips_optional_values_as_none() {
        let mut obj_file = empty_obj_file();
        obj_file.sections = vec![Section {
            name: "Section".to_string(),
            node: SourceNodeId(0),
            line_no: 0,
            data: SectionData::WRam0(RamSectionData { size: 0 }),
            mode: LinkingMode::Normal,
            address: None,
            bank: None,
            alignment: 0,
            alignment_offset: 0,
        }];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn round_trips_unicode_names() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![SourceNode {
            parent: None,
            parent_line_no: 0,
            data: SourceNodeData::File {
                name: "\u{1F600} \u{00E9}moji.asm".to_string(),
            },
            quieted: false,
        }];
        obj_file.symbols = vec![Symbol {
            name: "\u{30E9}\u{30D9}\u{30EB}".to_string(),
            data: SymbolData::Import,
        }];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn round_trips_when_symbol_and_section_counts_differ() {
        let mut obj_file = empty_obj_file();
        obj_file.source_nodes = vec![SourceNode {
            parent: None,
            parent_line_no: 0,
            data: SourceNodeData::File {
                name: "main.asm".to_string(),
            },
            quieted: false,
        }];
        obj_file.symbols = vec![
            Symbol {
                name: "One".to_string(),
                data: SymbolData::Import,
            },
            Symbol {
                name: "Two".to_string(),
                data: SymbolData::Import,
            },
            Symbol {
                name: "Three".to_string(),
                data: SymbolData::Import,
            },
        ];
        obj_file.sections = vec![Section {
            name: "OnlySection".to_string(),
            node: SourceNodeId(0),
            line_no: 0,
            data: SectionData::WRam0(RamSectionData { size: 1 }),
            mode: LinkingMode::Normal,
            address: None,
            bank: None,
            alignment: 0,
            alignment_offset: 0,
        }];
        assert_round_trips(&obj_file);
    }

    #[test]
    fn rejects_incorrect_magic_bytes() {
        let mut bytes = Vec::new();
        write(&mut bytes, &empty_obj_file()).unwrap();
        bytes[0] = b'X';
        let err = read(Cursor::new(bytes)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_unexpected_source_node_type() {
        let mut bytes = b"RGB9".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes()); // revision
        bytes.extend_from_slice(&0u32.to_le_bytes()); // symbols_count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // sections_count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // nodes_count
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // parent (none)
        bytes.extend_from_slice(&0u32.to_le_bytes()); // parent_line_no
        bytes.push(0x7F); // invalid node type
        let err = read(Cursor::new(bytes)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_unexpected_symbol_type() {
        let mut bytes = b"RGB9".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes()); // revision
        bytes.extend_from_slice(&1u32.to_le_bytes()); // symbols_count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // sections_count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // nodes_count
        bytes.push(b'S');
        bytes.push(0); // null terminator
        bytes.push(0xFF); // invalid symbol type
        let err = read(Cursor::new(bytes)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_rejects_string_with_null_byte() {
        let mut obj_file = empty_obj_file();
        obj_file.symbols = vec![Symbol {
            name: "bad\0name".to_string(),
            data: SymbolData::Import,
        }];
        let mut bytes = Vec::new();
        let err = write(&mut bytes, &obj_file).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn write_rejects_value_too_large_for_i32() {
        let mut obj_file = empty_obj_file();
        obj_file.revision = u32::MAX;
        let mut bytes = Vec::new();
        let err = write(&mut bytes, &obj_file).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
