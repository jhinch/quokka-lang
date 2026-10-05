/// `NOP`
pub const NOP: u8 = 0x00;
/// `STOP n8`
pub const STOP_N8: u8 = 0x10;
/// `JR NZ, e8`
pub const JR_NZ_E8: u8 = 0x20;
/// `JR NC, e8`
pub const JR_NC_E8: u8 = 0x30;

/// `LD BC, n16`
pub const LD16_BC_N16: u8 = 0x01;
/// `LD DE, n16`
pub const LD16_DE_N16: u8 = 0x11;
/// `LD HL, n16`
pub const LD16_HL_N16: u8 = 0x21;
/// `LD SP, n16`
pub const LD16_SP_N16: u8 = 0x31;

/// `LD [BC], A`
pub const LD_BC_A: u8 = 0x02;
/// `LD [DE], A`
pub const LD_DE_A: u8 = 0x12;
/// `LD [HLI], A`
pub const LD_HLI_A: u8 = 0x22;
/// `LD [HLD], A`
pub const LD_HLD_A: u8 = 0x32;

/// `INC BC`
pub const INC16_BC: u8 = 0x03;
/// `INC DE`
pub const INC16_DE: u8 = 0x13;
/// `INC HL`
pub const INC16_HL: u8 = 0x23;
/// `INC SP`
pub const INC16_SP: u8 = 0x33;

/// `INC B`
pub const INC_B: u8 = 0x04;
/// `INC D`
pub const INC_D: u8 = 0x14;
/// `INC H`
pub const INC_H: u8 = 0x24;
/// `INC [HL]`
pub const INC_HL: u8 = 0x34;

/// `DEC B`
pub const DEC_B: u8 = 0x05;
/// `DEC D`
pub const DEC_D: u8 = 0x15;
/// `DEC H`
pub const DEC_H: u8 = 0x25;
/// `DEC [HL]`
pub const DEC_HL: u8 = 0x35;

/// `LD B, n8`
pub const LD_B_N8: u8 = 0x06;
/// `LD D, n8`
pub const LD_D_N8: u8 = 0x16;
/// `LD H, n8`
pub const LD_H_N8: u8 = 0x26;
/// `LD [HL], n8`
pub const LD_HL_N8: u8 = 0x36;

/// `RLCA`
pub const RLCA: u8 = 0x07;
/// `RLA`
pub const RLA: u8 = 0x17;
/// `DAA`
pub const DAA: u8 = 0x27;
/// `SCF`
pub const SCF: u8 = 0x37;

/// `LD [A16], SP`
pub const LD_A16_SP: u8 = 0x08;
/// `JR e8`
pub const JR_E8: u8 = 0x18;
/// `JR Z, e8`
pub const JR_Z_E8: u8 = 0x28;
/// `JR C, e8`
pub const JR_C_E8: u8 = 0x38;

/// `ADD HL, BC`
pub const ADD_HL_BC: u8 = 0x09;
/// `ADD HL, DE`
pub const ADD_HL_DE: u8 = 0x19;
/// `ADD HL, HL`
pub const ADD_HL_HL: u8 = 0x29;
/// `ADD HL, SP`
pub const ADD_HL_SP: u8 = 0x39;

/// `LD A, [BC]`
pub const LD_A_BC: u8 = 0x0A;
/// `LD A, [DE]`
pub const LD_A_DE: u8 = 0x1A;
/// `LD A, [HLI]`
pub const LD_A_HLI: u8 = 0x2A;
/// `LD A, [HLD]`
pub const LD_A_HLD: u8 = 0x3A;

/// `DEC BC`
pub const DEC16_BC: u8 = 0x0B;
/// `DEC DE`
pub const DEC16_DE: u8 = 0x1B;
/// `DEC HL`
pub const DEC16_HL: u8 = 0x2B;
/// `DEC SP`
pub const DEC16_SP: u8 = 0x3B;

/// `INC C`
pub const INC_C: u8 = 0x0C;
/// `INC E`
pub const INC_E: u8 = 0x1C;
/// `INC L`
pub const INC_L: u8 = 0x2C;
/// `INC A`
pub const INC_A: u8 = 0x3C;

/// `DEC C`
pub const DEC_C: u8 = 0x0D;
/// `DEC E`
pub const DEC_E: u8 = 0x1D;
/// `DEC L`
pub const DEC_L: u8 = 0x2D;
/// `DEC A`
pub const DEC_A: u8 = 0x3D;

/// `LD C, n8`
pub const LD_C_N8: u8 = 0x0E;
/// `LD E, n8`
pub const LD_E_N8: u8 = 0x1E;
/// `LD L, n8`
pub const LD_L_N8: u8 = 0x2E;
/// `LD A, n8`
pub const LD_A_N8: u8 = 0x3E;

/// `RRCA`
pub const RRCA: u8 = 0x0F;
/// `RRA`
pub const RRA: u8 = 0x1F;
/// `CPL`
pub const CPL: u8 = 0x2F;
/// `CCF`
pub const CCF: u8 = 0x3F;

/// `LD B, B`
pub const LD_B_B: u8 = 0x40;
/// `LD B, C`
pub const LD_B_C: u8 = 0x41;
/// `LD B, D`
pub const LD_B_D: u8 = 0x42;
/// `LD B, E`
pub const LD_B_E: u8 = 0x43;
/// `LD B, H`
pub const LD_B_H: u8 = 0x44;
/// `LD B, L`
pub const LD_B_L: u8 = 0x45;
/// `LD B, [HL]`
pub const LD_B_HL: u8 = 0x46;
/// `LD B, A`
pub const LD_B_A: u8 = 0x47;

/// `LD C, B`
pub const LD_C_B: u8 = 0x48;
/// `LD C, C`
pub const LD_C_C: u8 = 0x49;
/// `LD C, D`
pub const LD_C_D: u8 = 0x4A;
/// `LD C, E`
pub const LD_C_E: u8 = 0x4B;
/// `LD C, H`
pub const LD_C_H: u8 = 0x4C;
/// `LD C, L`
pub const LD_C_L: u8 = 0x4D;
/// `LD C, [HL]`
pub const LD_C_HL: u8 = 0x4E;
/// `LD C, A`
pub const LD_C_A: u8 = 0x4F;

/// `LD D, B`
pub const LD_D_B: u8 = 0x50;
/// `LD D, C`
pub const LD_D_C: u8 = 0x51;
/// `LD D, D`
pub const LD_D_D: u8 = 0x52;
/// `LD D, E`
pub const LD_D_E: u8 = 0x53;
/// `LD D, H`
pub const LD_D_H: u8 = 0x54;
/// `LD D, L`
pub const LD_D_L: u8 = 0x55;
/// `LD D, [HL]`
pub const LD_D_HL: u8 = 0x56;
/// `LD D, A`
pub const LD_D_A: u8 = 0x57;

/// `LD E, B`
pub const LD_E_B: u8 = 0x58;
/// `LD E, C`
pub const LD_E_C: u8 = 0x59;
/// `LD E, D`
pub const LD_E_D: u8 = 0x5A;
/// `LD E, E`
pub const LD_E_E: u8 = 0x5B;
/// `LD E, H`
pub const LD_E_H: u8 = 0x5C;
/// `LD E, L`
pub const LD_E_L: u8 = 0x5D;
/// `LD E, [HL]`
pub const LD_E_HL: u8 = 0x5E;
/// `LD E, A`
pub const LD_E_A: u8 = 0x5F;

/// `LD H, B`
pub const LD_H_B: u8 = 0x60;
/// `LD H, C`
pub const LD_H_C: u8 = 0x61;
/// `LD H, D`
pub const LD_H_D: u8 = 0x62;
/// `LD H, E`
pub const LD_H_E: u8 = 0x63;
/// `LD H, H`
pub const LD_H_H: u8 = 0x64;
/// `LD H, L`
pub const LD_H_L: u8 = 0x65;
/// `LD H, [HL]`
pub const LD_H_HL: u8 = 0x66;
/// `LD H, A`
pub const LD_H_A: u8 = 0x67;

/// `LD L, B`
pub const LD_L_B: u8 = 0x68;
/// `LD L, C`
pub const LD_L_C: u8 = 0x69;
/// `LD L, D`
pub const LD_L_D: u8 = 0x6A;
/// `LD L, E`
pub const LD_L_E: u8 = 0x6B;
/// `LD L, H`
pub const LD_L_H: u8 = 0x6C;
/// `LD L, L`
pub const LD_L_L: u8 = 0x6D;
/// `LD L, [HL]`
pub const LD_L_HL: u8 = 0x6E;
/// `LD L, A`
pub const LD_L_A: u8 = 0x6F;

/// `LD [HL], B`
pub const LD_HL_B: u8 = 0x70;
/// `LD [HL], C`
pub const LD_HL_C: u8 = 0x71;
/// `LD [HL], D`
pub const LD_HL_D: u8 = 0x72;
/// `LD [HL], E`
pub const LD_HL_E: u8 = 0x73;
/// `LD [HL], H`
pub const LD_HL_H: u8 = 0x74;
/// `LD [HL], L`
pub const LD_HL_L: u8 = 0x75;
/// `HALT`
pub const HALT: u8 = 0x76;
/// `LD [HL], A`
pub const LD_HL_A: u8 = 0x77;

/// `LD A, B`
pub const LD_A_B: u8 = 0x78;
/// `LD A, C`
pub const LD_A_C: u8 = 0x79;
/// `LD A, D`
pub const LD_A_D: u8 = 0x7A;
/// `LD A, E`
pub const LD_A_E: u8 = 0x7B;
/// `LD A, H`
pub const LD_A_H: u8 = 0x7C;
/// `LD A, L`
pub const LD_A_L: u8 = 0x7D;
/// `LD A, [HL]`
pub const LD_A_HL: u8 = 0x7E;
/// `LD A, A`
pub const LD_A_A: u8 = 0x7F;

/// `ADD A, B`
pub const ADD_A_B: u8 = 0x80;
/// `ADD A, C`
pub const ADD_A_C: u8 = 0x81;
/// `ADD A, D`
pub const ADD_A_D: u8 = 0x82;
/// `ADD A, E`
pub const ADD_A_E: u8 = 0x83;
/// `ADD A, H`
pub const ADD_A_H: u8 = 0x84;
/// `ADD A, L`
pub const ADD_A_L: u8 = 0x85;
/// `ADD A, [HL]`
pub const ADD_A_HL: u8 = 0x86;
/// `ADD A, A`
pub const ADD_A_A: u8 = 0x87;

/// `ADC A, B`
pub const ADC_A_B: u8 = 0x88;
/// `ADC A, C`
pub const ADC_A_C: u8 = 0x89;
/// `ADC A, D`
pub const ADC_A_D: u8 = 0x8A;
/// `ADC A, E`
pub const ADC_A_E: u8 = 0x8B;
/// `ADC A, H`
pub const ADC_A_H: u8 = 0x8C;
/// `ADC A, L`
pub const ADC_A_L: u8 = 0x8D;
/// `ADC A, [HL]`
pub const ADC_A_HL: u8 = 0x8E;
/// `ADC A, A`
pub const ADC_A_A: u8 = 0x8F;

/// `SUB A, B`
pub const SUB_A_B: u8 = 0x90;
/// `SUB A, C`
pub const SUB_A_C: u8 = 0x91;
/// `SUB A, D`
pub const SUB_A_D: u8 = 0x92;
/// `SUB A, E`
pub const SUB_A_E: u8 = 0x93;
/// `SUB A, H`
pub const SUB_A_H: u8 = 0x94;
/// `SUB A, L`
pub const SUB_A_L: u8 = 0x95;
/// `SUB A, [HL]`
pub const SUB_A_HL: u8 = 0x96;
/// `SUB A, A`
pub const SUB_A_A: u8 = 0x97;

/// `SBC A, B`
pub const SBC_A_B: u8 = 0x98;
/// `SBC A, C`
pub const SBC_A_C: u8 = 0x99;
/// `SBC A, D`
pub const SBC_A_D: u8 = 0x9A;
/// `SBC A, E`
pub const SBC_A_E: u8 = 0x9B;
/// `SBC A, H`
pub const SBC_A_H: u8 = 0x9C;
/// `SBC A, L`
pub const SBC_A_L: u8 = 0x9D;
/// `SBC A, [HL]`
pub const SBC_A_HL: u8 = 0x9E;
/// `SBC A, A`
pub const SBC_A_A: u8 = 0x9F;

/// `AND A, B`
pub const AND_A_B: u8 = 0xA0;
/// `AND A, C`
pub const AND_A_C: u8 = 0xA1;
/// `AND A, D`
pub const AND_A_D: u8 = 0xA2;
/// `AND A, E`
pub const AND_A_E: u8 = 0xA3;
/// `AND A, H`
pub const AND_A_H: u8 = 0xA4;
/// `AND A, L`
pub const AND_A_L: u8 = 0xA5;
/// `AND A, [HL]`
pub const AND_A_HL: u8 = 0xA6;
/// `AND A, A`
pub const AND_A_A: u8 = 0xA7;

/// `XOR A, B`
pub const XOR_A_B: u8 = 0xA8;
/// `XOR A, C`
pub const XOR_A_C: u8 = 0xA9;
/// `XOR A, D`
pub const XOR_A_D: u8 = 0xAA;
/// `XOR A, E`
pub const XOR_A_E: u8 = 0xAB;
/// `XOR A, H`
pub const XOR_A_H: u8 = 0xAC;
/// `XOR A, L`
pub const XOR_A_L: u8 = 0xAD;
/// `XOR A, [HL]`
pub const XOR_A_HL: u8 = 0xAE;
/// `XOR A, A`
pub const XOR_A_A: u8 = 0xAF;

/// `OR A, B`
pub const OR_A_B: u8 = 0xB0;
/// `OR A, C`
pub const OR_A_C: u8 = 0xB1;
/// `OR A, D`
pub const OR_A_D: u8 = 0xB2;
/// `OR A, E`
pub const OR_A_E: u8 = 0xB3;
/// `OR A, H`
pub const OR_A_H: u8 = 0xB4;
/// `OR A, L`
pub const OR_A_L: u8 = 0xB5;
/// `OR A, [HL]`
pub const OR_A_HL: u8 = 0xB6;
/// `OR A, A`
pub const OR_A_A: u8 = 0xB7;

/// `CP A, B`
pub const CP_A_B: u8 = 0xB8;
/// `CP A, C`
pub const CP_A_C: u8 = 0xB9;
/// `CP A, D`
pub const CP_A_D: u8 = 0xBA;
/// `CP A, E`
pub const CP_A_E: u8 = 0xBB;
/// `CP A, H`
pub const CP_A_H: u8 = 0xBC;
/// `CP A, L`
pub const CP_A_L: u8 = 0xBD;
/// `CP A, [HL]`
pub const CP_A_HL: u8 = 0xBE;
/// `CP A, A`
pub const CP_A_A: u8 = 0xBF;

/// `RET NZ`
pub const RET_NZ: u8 = 0xC0;
/// `RET NC`
pub const RET_NC: u8 = 0xD0;
/// `LDH [a8], A`
pub const LDH_A8_A: u8 = 0xE0;
/// `LDH A, [a8]`
pub const LDH_A_A8: u8 = 0xF0;

/// `POP BC`
pub const POP_BC: u8 = 0xC1;
/// `POP DE`
pub const POP_DE: u8 = 0xD1;
/// `POP HL`
pub const POP_HL: u8 = 0xE1;
/// `POP AF`
pub const POP_AF: u8 = 0xF1;

/// `JP NZ, a16`
pub const JP_NZ_A16: u8 = 0xC2;
/// `JP NC, a16`
pub const JP_NC_A16: u8 = 0xD2;
/// `LDH [C], A`
pub const LDH_C_A: u8 = 0xE2;
/// `LDH A, [C]`
pub const LDH_A_C: u8 = 0xF2;

/// `JP a16`
pub const JP_A16: u8 = 0xC3;
/// `DI`
pub const DI: u8 = 0xF3;

/// `CALL NZ, a16`
pub const CALL_NZ_A16: u8 = 0xC4;
/// `CALL NC, a16`
pub const CALL_NC_A16: u8 = 0xD4;

/// `PUSH BC`
pub const PUSH_BC: u8 = 0xC5;
/// `PUSH DE`
pub const PUSH_DE: u8 = 0xD5;
/// `PUSH HL`
pub const PUSH_HL: u8 = 0xE5;
/// `PUSH AF`
pub const PUSH_AF: u8 = 0xF5;

/// `ADD A, n8`
pub const ADD_A_N8: u8 = 0xC6;
/// `SUB A, n8`
pub const SUB_A_N8: u8 = 0xD6;
/// `AND A, n8`
pub const AND_A_N8: u8 = 0xE6;
/// `OR A, n8`
pub const OR_A_N8: u8 = 0xF6;

/// `RST $00`
pub const RST_00: u8 = 0xC7;
/// `RST $10`
pub const RST_10: u8 = 0xD7;
/// `RST $20`
pub const RST_20: u8 = 0xE7;
/// `RST $30`
pub const RST_30: u8 = 0xF7;

/// `RET Z`
pub const RET_Z: u8 = 0xC8;
/// `RET C`
pub const RET_C: u8 = 0xD8;
/// `ADD SP, e8`
pub const ADD_SP_E8: u8 = 0xE8;
/// `LD HL, SP+e8`
pub const LD_HL_SP_E8: u8 = 0xF8;

/// `RET`
pub const RET: u8 = 0xC9;
/// `RETI`
pub const RETI: u8 = 0xD9;
/// `JP HL`
pub const JP_HL: u8 = 0xE9;
/// `LD SP, HL`
pub const LD_SP_HL: u8 = 0xF9;

/// `JP Z, a16`
pub const JP_Z_A16: u8 = 0xCA;
/// `JP C, a16`
pub const JP_C_A16: u8 = 0xDA;
/// `LD [a16], A`
pub const LD_A16_A: u8 = 0xEA;
/// `LD A, [a16]`
pub const LD_A_A16: u8 = 0xFA;

/// `EI`
pub const EI: u8 = 0xFB;

/// `CALL Z, a16`
pub const CALL_Z_A16: u8 = 0xCC;
/// `CALL C, a16`
pub const CALL_C_A16: u8 = 0xDC;

/// `CALL a16`
pub const CALL_A16: u8 = 0xCD;

/// `ADC A, n8`
pub const ADC_A_N8: u8 = 0xCE;
/// `SBC A, n8`
pub const SBC_A_N8: u8 = 0xDE;
/// `XOR A, n8`
pub const XOR_A_N8: u8 = 0xEE;
/// `CP A, n8`
pub const CP_A_N8: u8 = 0xFE;

/// `RST $08`
pub const RST_08: u8 = 0xCF;
/// `RST $18`
pub const RST_18: u8 = 0xDF;
/// `RST $28`
pub const RST_28: u8 = 0xEF;
/// `RST $38`
pub const RST_38: u8 = 0xFF;

/// `RLC B`
pub const RLC_B: u16 = 0x00CB;
/// `RLC C`
pub const RLC_C: u16 = 0x01CB;
/// `RLC D`
pub const RLC_D: u16 = 0x02CB;
/// `RLC E`
pub const RLC_E: u16 = 0x03CB;
/// `RLC H`
pub const RLC_H: u16 = 0x04CB;
/// `RLC L`
pub const RLC_L: u16 = 0x05CB;
/// `RLC [HL]`
pub const RLC_HL: u16 = 0x06CB;
/// `RLC A`
pub const RLC_A: u16 = 0x07CB;

/// `RRC B`
pub const RRC_B: u16 = 0x08CB;
/// `RRC C`
pub const RRC_C: u16 = 0x09CB;
/// `RRC D`
pub const RRC_D: u16 = 0x0ACB;
/// `RRC E`
pub const RRC_E: u16 = 0x0BCB;
/// `RRC H`
pub const RRC_H: u16 = 0x0CCB;
/// `RRC L`
pub const RRC_L: u16 = 0x0DCB;
/// `RRC [HL]`
pub const RRC_HL: u16 = 0x0ECB;
/// `RRC A`
pub const RRC_A: u16 = 0x0FCB;

/// `RL B`
pub const RL_B: u16 = 0x10CB;
/// `RL C`
pub const RL_C: u16 = 0x11CB;
/// `RL D`
pub const RL_D: u16 = 0x12CB;
/// `RL E`
pub const RL_E: u16 = 0x13CB;
/// `RL H`
pub const RL_H: u16 = 0x14CB;
/// `RL L`
pub const RL_L: u16 = 0x15CB;
/// `RL [HL]`
pub const RL_HL: u16 = 0x16CB;
/// `RL A`
pub const RL_A: u16 = 0x17CB;

/// `RR B`
pub const RR_B: u16 = 0x18CB;
/// `RR C`
pub const RR_C: u16 = 0x19CB;
/// `RR D`
pub const RR_D: u16 = 0x1ACB;
/// `RR E`
pub const RR_E: u16 = 0x1BCB;
/// `RR H`
pub const RR_H: u16 = 0x1CCB;
/// `RR L`
pub const RR_L: u16 = 0x1DCB;
/// `RR [HL]`
pub const RR_HL: u16 = 0x1ECB;
/// `RR A`
pub const RR_A: u16 = 0x1FCB;

/// `SLA B`
pub const SLA_B: u16 = 0x20CB;
/// `SLA C`
pub const SLA_C: u16 = 0x21CB;
/// `SLA D`
pub const SLA_D: u16 = 0x22CB;
/// `SLA E`
pub const SLA_E: u16 = 0x23CB;
/// `SLA H`
pub const SLA_H: u16 = 0x24CB;
/// `SLA L`
pub const SLA_L: u16 = 0x25CB;
/// `SLA [HL]`
pub const SLA_HL: u16 = 0x26CB;
/// `SLA A`
pub const SLA_A: u16 = 0x27CB;

/// `SRA B`
pub const SRA_B: u16 = 0x28CB;
/// `SRA C`
pub const SRA_C: u16 = 0x29CB;
/// `SRA D`
pub const SRA_D: u16 = 0x2ACB;
/// `SRA E`
pub const SRA_E: u16 = 0x2BCB;
/// `SRA H`
pub const SRA_H: u16 = 0x2CCB;
/// `SRA L`
pub const SRA_L: u16 = 0x2DCB;
/// `SRA [HL]`
pub const SRA_HL: u16 = 0x2ECB;
/// `SRA A`
pub const SRA_A: u16 = 0x2FCB;

/// `SWAP B`
pub const SWAP_B: u16 = 0x30CB;
/// `SWAP C`
pub const SWAP_C: u16 = 0x31CB;
/// `SWAP D`
pub const SWAP_D: u16 = 0x32CB;
/// `SWAP E`
pub const SWAP_E: u16 = 0x33CB;
/// `SWAP H`
pub const SWAP_H: u16 = 0x34CB;
/// `SWAP L`
pub const SWAP_L: u16 = 0x35CB;
/// `SWAP [HL]`
pub const SWAP_HL: u16 = 0x36CB;
/// `SWAP A`
pub const SWAP_A: u16 = 0x37CB;

/// `SRL B`
pub const SRL_B: u16 = 0x38CB;
/// `SRL C`
pub const SRL_C: u16 = 0x39CB;
/// `SRL D`
pub const SRL_D: u16 = 0x3ACB;
/// `SRL E`
pub const SRL_E: u16 = 0x3BCB;
/// `SRL H`
pub const SRL_H: u16 = 0x3CCB;
/// `SRL L`
pub const SRL_L: u16 = 0x3DCB;
/// `SRL [HL]`
pub const SRL_HL: u16 = 0x3ECB;
/// `SRL A`
pub const SRL_A: u16 = 0x3FCB;

/// `BIT 0, B`
pub const BIT_0_B: u16 = 0x40CB;
/// `BIT 0, C`
pub const BIT_0_C: u16 = 0x41CB;
/// `BIT 0, D`
pub const BIT_0_D: u16 = 0x42CB;
/// `BIT 0, E`
pub const BIT_0_E: u16 = 0x43CB;
/// `BIT 0, H`
pub const BIT_0_H: u16 = 0x44CB;
/// `BIT 0, L`
pub const BIT_0_L: u16 = 0x45CB;
/// `BIT 0, [HL]`
pub const BIT_0_HL: u16 = 0x46CB;
/// `BIT 0, A`
pub const BIT_0_A: u16 = 0x47CB;

/// `BIT 1, B`
pub const BIT_1_B: u16 = 0x48CB;
/// `BIT 1, C`
pub const BIT_1_C: u16 = 0x49CB;
/// `BIT 1, D`
pub const BIT_1_D: u16 = 0x4ACB;
/// `BIT 1, E`
pub const BIT_1_E: u16 = 0x4BCB;
/// `BIT 1, H`
pub const BIT_1_H: u16 = 0x4CCB;
/// `BIT 1, L`
pub const BIT_1_L: u16 = 0x4DCB;
/// `BIT 1, [HL]`
pub const BIT_1_HL: u16 = 0x4ECB;
/// `BIT 1, A`
pub const BIT_1_A: u16 = 0x4FCB;

/// `BIT 2, B`
pub const BIT_2_B: u16 = 0x50CB;
/// `BIT 2, C`
pub const BIT_2_C: u16 = 0x51CB;
/// `BIT 2, D`
pub const BIT_2_D: u16 = 0x52CB;
/// `BIT 2, E`
pub const BIT_2_E: u16 = 0x53CB;
/// `BIT 2, H`
pub const BIT_2_H: u16 = 0x54CB;
/// `BIT 2, L`
pub const BIT_2_L: u16 = 0x55CB;
/// `BIT 2, [HL]`
pub const BIT_2_HL: u16 = 0x56CB;
/// `BIT 2, A`
pub const BIT_2_A: u16 = 0x57CB;

/// `BIT 3, B`
pub const BIT_3_B: u16 = 0x58CB;
/// `BIT 3, C`
pub const BIT_3_C: u16 = 0x59CB;
/// `BIT 3, D`
pub const BIT_3_D: u16 = 0x5ACB;
/// `BIT 3, E`
pub const BIT_3_E: u16 = 0x5BCB;
/// `BIT 3, H`
pub const BIT_3_H: u16 = 0x5CCB;
/// `BIT 3, L`
pub const BIT_3_L: u16 = 0x5DCB;
/// `BIT 3, [HL]`
pub const BIT_3_HL: u16 = 0x5ECB;
/// `BIT 3, A`
pub const BIT_3_A: u16 = 0x5FCB;

/// `BIT 4, B`
pub const BIT_4_B: u16 = 0x60CB;
/// `BIT 4, C`
pub const BIT_4_C: u16 = 0x61CB;
/// `BIT 4, D`
pub const BIT_4_D: u16 = 0x62CB;
/// `BIT 4, E`
pub const BIT_4_E: u16 = 0x63CB;
/// `BIT 4, H`
pub const BIT_4_H: u16 = 0x64CB;
/// `BIT 4, L`
pub const BIT_4_L: u16 = 0x65CB;
/// `BIT 4, [HL]`
pub const BIT_4_HL: u16 = 0x66CB;
/// `BIT 4, A`
pub const BIT_4_A: u16 = 0x67CB;

/// `BIT 5, B`
pub const BIT_5_B: u16 = 0x68CB;
/// `BIT 5, C`
pub const BIT_5_C: u16 = 0x69CB;
/// `BIT 5, D`
pub const BIT_5_D: u16 = 0x6ACB;
/// `BIT 5, E`
pub const BIT_5_E: u16 = 0x6BCB;
/// `BIT 5, H`
pub const BIT_5_H: u16 = 0x6CCB;
/// `BIT 5, L`
pub const BIT_5_L: u16 = 0x6DCB;
/// `BIT 5, [HL]`
pub const BIT_5_HL: u16 = 0x6ECB;
/// `BIT 5, A`
pub const BIT_5_A: u16 = 0x6FCB;

/// `BIT 6, B`
pub const BIT_6_B: u16 = 0x70CB;
/// `BIT 6, C`
pub const BIT_6_C: u16 = 0x71CB;
/// `BIT 6, D`
pub const BIT_6_D: u16 = 0x72CB;
/// `BIT 6, E`
pub const BIT_6_E: u16 = 0x73CB;
/// `BIT 6, H`
pub const BIT_6_H: u16 = 0x74CB;
/// `BIT 6, L`
pub const BIT_6_L: u16 = 0x75CB;
/// `BIT 6, [HL]`
pub const BIT_6_HL: u16 = 0x76CB;
/// `BIT 6, A`
pub const BIT_6_A: u16 = 0x77CB;

/// `BIT 7, B`
pub const BIT_7_B: u16 = 0x78CB;
/// `BIT 7, C`
pub const BIT_7_C: u16 = 0x79CB;
/// `BIT 7, D`
pub const BIT_7_D: u16 = 0x7ACB;
/// `BIT 7, E`
pub const BIT_7_E: u16 = 0x7BCB;
/// `BIT 7, H`
pub const BIT_7_H: u16 = 0x7CCB;
/// `BIT 7, L`
pub const BIT_7_L: u16 = 0x7DCB;
/// `BIT 7, [HL]`
pub const BIT_7_HL: u16 = 0x7ECB;
/// `BIT 7, A`
pub const BIT_7_A: u16 = 0x7FCB;

/// `RES 0, B`
pub const RES_0_B: u16 = 0x80CB;
/// `RES 0, C`
pub const RES_0_C: u16 = 0x81CB;
/// `RES 0, D`
pub const RES_0_D: u16 = 0x82CB;
/// `RES 0, E`
pub const RES_0_E: u16 = 0x83CB;
/// `RES 0, H`
pub const RES_0_H: u16 = 0x84CB;
/// `RES 0, L`
pub const RES_0_L: u16 = 0x85CB;
/// `RES 0, [HL]`
pub const RES_0_HL: u16 = 0x86CB;
/// `RES 0, A`
pub const RES_0_A: u16 = 0x87CB;

/// `RES 1, B`
pub const RES_1_B: u16 = 0x88CB;
/// `RES 1, C`
pub const RES_1_C: u16 = 0x89CB;
/// `RES 1, D`
pub const RES_1_D: u16 = 0x8ACB;
/// `RES 1, E`
pub const RES_1_E: u16 = 0x8BCB;
/// `RES 1, H`
pub const RES_1_H: u16 = 0x8CCB;
/// `RES 1, L`
pub const RES_1_L: u16 = 0x8DCB;
/// `RES 1, [HL]`
pub const RES_1_HL: u16 = 0x8ECB;
/// `RES 1, A`
pub const RES_1_A: u16 = 0x8FCB;

/// `RES 2, B`
pub const RES_2_B: u16 = 0x90CB;
/// `RES 2, C`
pub const RES_2_C: u16 = 0x91CB;
/// `RES 2, D`
pub const RES_2_D: u16 = 0x92CB;
/// `RES 2, E`
pub const RES_2_E: u16 = 0x93CB;
/// `RES 2, H`
pub const RES_2_H: u16 = 0x94CB;
/// `RES 2, L`
pub const RES_2_L: u16 = 0x95CB;
/// `RES 2, [HL]`
pub const RES_2_HL: u16 = 0x96CB;
/// `RES 2, A`
pub const RES_2_A: u16 = 0x97CB;

/// `RES 3, B`
pub const RES_3_B: u16 = 0x98CB;
/// `RES 3, C`
pub const RES_3_C: u16 = 0x99CB;
/// `RES 3, D`
pub const RES_3_D: u16 = 0x9ACB;
/// `RES 3, E`
pub const RES_3_E: u16 = 0x9BCB;
/// `RES 3, H`
pub const RES_3_H: u16 = 0x9CCB;
/// `RES 3, L`
pub const RES_3_L: u16 = 0x9DCB;
/// `RES 3, [HL]`
pub const RES_3_HL: u16 = 0x9ECB;
/// `RES 3, A`
pub const RES_3_A: u16 = 0x9FCB;

/// `RES 4, B`
pub const RES_4_B: u16 = 0xA0CB;
/// `RES 4, C`
pub const RES_4_C: u16 = 0xA1CB;
/// `RES 4, D`
pub const RES_4_D: u16 = 0xA2CB;
/// `RES 4, E`
pub const RES_4_E: u16 = 0xA3CB;
/// `RES 4, H`
pub const RES_4_H: u16 = 0xA4CB;
/// `RES 4, L`
pub const RES_4_L: u16 = 0xA5CB;
/// `RES 4, [HL]`
pub const RES_4_HL: u16 = 0xA6CB;
/// `RES 4, A`
pub const RES_4_A: u16 = 0xA7CB;

/// `RES 5, B`
pub const RES_5_B: u16 = 0xA8CB;
/// `RES 5, C`
pub const RES_5_C: u16 = 0xA9CB;
/// `RES 5, D`
pub const RES_5_D: u16 = 0xAACB;
/// `RES 5, E`
pub const RES_5_E: u16 = 0xABCB;
/// `RES 5, H`
pub const RES_5_H: u16 = 0xACCB;
/// `RES 5, L`
pub const RES_5_L: u16 = 0xADCB;
/// `RES 5, [HL]`
pub const RES_5_HL: u16 = 0xAECB;
/// `RES 5, A`
pub const RES_5_A: u16 = 0xAFCB;

/// `RES 6, B`
pub const RES_6_B: u16 = 0xB0CB;
/// `RES 6, C`
pub const RES_6_C: u16 = 0xB1CB;
/// `RES 6, D`
pub const RES_6_D: u16 = 0xB2CB;
/// `RES 6, E`
pub const RES_6_E: u16 = 0xB3CB;
/// `RES 6, H`
pub const RES_6_H: u16 = 0xB4CB;
/// `RES 6, L`
pub const RES_6_L: u16 = 0xB5CB;
/// `RES 6, [HL]`
pub const RES_6_HL: u16 = 0xB6CB;
/// `RES 6, A`
pub const RES_6_A: u16 = 0xB7CB;

/// `RES 7, B`
pub const RES_7_B: u16 = 0xB8CB;
/// `RES 7, C`
pub const RES_7_C: u16 = 0xB9CB;
/// `RES 7, D`
pub const RES_7_D: u16 = 0xBACB;
/// `RES 7, E`
pub const RES_7_E: u16 = 0xBBCB;
/// `RES 7, H`
pub const RES_7_H: u16 = 0xBCCB;
/// `RES 7, L`
pub const RES_7_L: u16 = 0xBDCB;
/// `RES 7, [HL]`
pub const RES_7_HL: u16 = 0xBECB;
/// `RES 7, A`
pub const RES_7_A: u16 = 0xBFCB;

/// `SET 0, B`
pub const SET_0_B: u16 = 0xC0CB;
/// `SET 0, C`
pub const SET_0_C: u16 = 0xC1CB;
/// `SET 0, D`
pub const SET_0_D: u16 = 0xC2CB;
/// `SET 0, E`
pub const SET_0_E: u16 = 0xC3CB;
/// `SET 0, H`
pub const SET_0_H: u16 = 0xC4CB;
/// `SET 0, L`
pub const SET_0_L: u16 = 0xC5CB;
/// `SET 0, [HL]`
pub const SET_0_HL: u16 = 0xC6CB;
/// `SET 0, A`
pub const SET_0_A: u16 = 0xC7CB;

/// `SET 1, B`
pub const SET_1_B: u16 = 0xC8CB;
/// `SET 1, C`
pub const SET_1_C: u16 = 0xC9CB;
/// `SET 1, D`
pub const SET_1_D: u16 = 0xCACB;
/// `SET 1, E`
pub const SET_1_E: u16 = 0xCBCB;
/// `SET 1, H`
pub const SET_1_H: u16 = 0xCCCB;
/// `SET 1, L`
pub const SET_1_L: u16 = 0xCDCB;
/// `SET 1, [HL]`
pub const SET_1_HL: u16 = 0xCECB;
/// `SET 1, A`
pub const SET_1_A: u16 = 0xCFCB;

/// `SET 2, B`
pub const SET_2_B: u16 = 0xD0CB;
/// `SET 2, C`
pub const SET_2_C: u16 = 0xD1CB;
/// `SET 2, D`
pub const SET_2_D: u16 = 0xD2CB;
/// `SET 2, E`
pub const SET_2_E: u16 = 0xD3CB;
/// `SET 2, H`
pub const SET_2_H: u16 = 0xD4CB;
/// `SET 2, L`
pub const SET_2_L: u16 = 0xD5CB;
/// `SET 2, [HL]`
pub const SET_2_HL: u16 = 0xD6CB;
/// `SET 2, A`
pub const SET_2_A: u16 = 0xD7CB;

/// `SET 3, B`
pub const SET_3_B: u16 = 0xD8CB;
/// `SET 3, C`
pub const SET_3_C: u16 = 0xD9CB;
/// `SET 3, D`
pub const SET_3_D: u16 = 0xDACB;
/// `SET 3, E`
pub const SET_3_E: u16 = 0xDBCB;
/// `SET 3, H`
pub const SET_3_H: u16 = 0xDCCB;
/// `SET 3, L`
pub const SET_3_L: u16 = 0xDDCB;
/// `SET 3, [HL]`
pub const SET_3_HL: u16 = 0xDECB;
/// `SET 3, A`
pub const SET_3_A: u16 = 0xDFCB;

/// `SET 4, B`
pub const SET_4_B: u16 = 0xE0CB;
/// `SET 4, C`
pub const SET_4_C: u16 = 0xE1CB;
/// `SET 4, D`
pub const SET_4_D: u16 = 0xE2CB;
/// `SET 4, E`
pub const SET_4_E: u16 = 0xE3CB;
/// `SET 4, H`
pub const SET_4_H: u16 = 0xE4CB;
/// `SET 4, L`
pub const SET_4_L: u16 = 0xE5CB;
/// `SET 4, [HL]`
pub const SET_4_HL: u16 = 0xE6CB;
/// `SET 4, A`
pub const SET_4_A: u16 = 0xE7CB;

/// `SET 5, B`
pub const SET_5_B: u16 = 0xE8CB;
/// `SET 5, C`
pub const SET_5_C: u16 = 0xE9CB;
/// `SET 5, D`
pub const SET_5_D: u16 = 0xEACB;
/// `SET 5, E`
pub const SET_5_E: u16 = 0xEBCB;
/// `SET 5, H`
pub const SET_5_H: u16 = 0xECCB;
/// `SET 5, L`
pub const SET_5_L: u16 = 0xEDCB;
/// `SET 5, [HL]`
pub const SET_5_HL: u16 = 0xEECB;
/// `SET 5, A`
pub const SET_5_A: u16 = 0xEFCB;

/// `SET 6, B`
pub const SET_6_B: u16 = 0xF0CB;
/// `SET 6, C`
pub const SET_6_C: u16 = 0xF1CB;
/// `SET 6, D`
pub const SET_6_D: u16 = 0xF2CB;
/// `SET 6, E`
pub const SET_6_E: u16 = 0xF3CB;
/// `SET 6, H`
pub const SET_6_H: u16 = 0xF4CB;
/// `SET 6, L`
pub const SET_6_L: u16 = 0xF5CB;
/// `SET 6, [HL]`
pub const SET_6_HL: u16 = 0xF6CB;
/// `SET 6, A`
pub const SET_6_A: u16 = 0xF7CB;

/// `SET 7, B`
pub const SET_7_B: u16 = 0xF8CB;
/// `SET 7, C`
pub const SET_7_C: u16 = 0xF9CB;
/// `SET 7, D`
pub const SET_7_D: u16 = 0xFACB;
/// `SET 7, E`
pub const SET_7_E: u16 = 0xFBCB;
/// `SET 7, H`
pub const SET_7_H: u16 = 0xFCCB;
/// `SET 7, L`
pub const SET_7_L: u16 = 0xFDCB;
/// `SET 7, [HL]`
pub const SET_7_HL: u16 = 0xFECB;
/// `SET 7, A`
pub const SET_7_A: u16 = 0xFFCB;

/// The encoded length in bytes of the instruction starting with `opcode`.
#[must_use]
pub const fn instruction_len(opcode: u8) -> u8 {
    match opcode {
        // LD rr, n16
        LD16_BC_N16 | LD16_DE_N16 | LD16_HL_N16 | LD16_SP_N16
        // LD [a16], SP
        | LD_A16_SP
        // JP a16 / JP cc, a16
        | JP_NZ_A16 | JP_A16 | JP_Z_A16 | JP_NC_A16 | JP_C_A16
        // CALL a16 / CALL cc, a16
        | CALL_NZ_A16 | CALL_Z_A16 | CALL_A16 | CALL_NC_A16 | CALL_C_A16
        // LD [a16], A / LD A, [a16]
        | LD_A16_A | LD_A_A16 => 3,
        // LD r, n8
        LD_B_N8 | LD_C_N8 | LD_D_N8 | LD_E_N8 | LD_H_N8 | LD_L_N8 | LD_HL_N8 | LD_A_N8
        // STOP n8
        | STOP_N8
        // JR e8 / JR cc, e8
        | JR_E8 | JR_NZ_E8 | JR_Z_E8 | JR_NC_E8 | JR_C_E8
        // ALU A, n8
        | ADD_A_N8 | ADC_A_N8 | SUB_A_N8 | SBC_A_N8 | AND_A_N8 | XOR_A_N8 | OR_A_N8 | CP_A_N8
        // CB prefix
        | 0xCB
        // LDH [a8], A / LDH A, [a8]
        | LDH_A8_A | LDH_A_A8
        // ADD SP, e8 / LD HL, SP+e8
        | ADD_SP_E8 | LD_HL_SP_E8 => 2,
        _ => 1,
    }
}
