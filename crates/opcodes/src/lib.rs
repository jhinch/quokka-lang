//! The opcodes of the Game Boy's SM83 CPU, along with how long each instruction is and how long
//! it takes.
//!
//! Everything here is generated at build time from the `Opcodes.json` of
//! [gb-opcodes](https://github.com/gbdev/gb-opcodes), a copy of which lives in `gb-opcodes/`.
//!
//! A constant is named after its instruction's mnemonic and operands. An operand which is
//! dereferenced has a `_PTR` suffix, with an `I` or a `D` before it if the register is
//! incremented or decremented afterwards: `LD [HL], A` is [`LD_HL_PTR_A`] and `LD A, [HLI]` is
//! [`LD_A_HLI_PTR`].
//!
//! An unprefixed opcode is a `u8`. A `CB`-prefixed one is a `u16` holding both of its bytes, in
//! the order `to_le_bytes` gives them: [`BIT_0_B`] is `0x40CB`, encoded as `CB 40`.

#![no_std]

include!(concat!(env!("OUT_DIR"), "/opcodes.rs"));

/// How many M-cycles an instruction takes. The two differ only for a conditional jump, call or
/// return, which takes `min` when the condition does not hold and `max` when it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cycles {
    pub min: u8,
    pub max: u8,
}

/// The encoded length in bytes of the instruction starting with `opcode`. For [`PREFIX`] this
/// counts the byte following it.
#[must_use]
pub const fn instruction_len(opcode: u8) -> u8 {
    INSTRUCTION_LEN[opcode as usize]
}

/// How long the instruction starting with `opcode` takes.
///
/// For [`PREFIX`] this is only the fetch of the prefix itself: [`prefixed_cycles`] gives the
/// whole of a prefixed instruction. The opcodes which are not instructions lock up the CPU, and
/// are counted as their fetch alone.
#[must_use]
pub const fn cycles(opcode: u8) -> Cycles {
    CYCLES[opcode as usize]
}

/// How long the prefixed instruction `opcode` takes, with the fetch of its prefix included.
#[must_use]
pub const fn prefixed_cycles(opcode: u16) -> Cycles {
    let [prefix, opcode] = opcode.to_le_bytes();
    assert!(prefix == PREFIX, "not a prefixed opcode");
    PREFIXED_CYCLES[opcode as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcodes() {
        assert_eq!(NOP, 0x00);
        assert_eq!(LD_HL_N16, 0x21);
        assert_eq!(LD_HL_PTR_N8, 0x36);
        assert_eq!(LD_A_HLI_PTR, 0x2A);
        assert_eq!(LD_HLD_PTR_A, 0x32);
        assert_eq!(LD_A16_PTR_SP, 0x08);
        assert_eq!(LDH_A8_PTR_A, 0xE0);
        assert_eq!(LDH_A_C_PTR, 0xF2);
        assert_eq!(LD_HL_SP_E8, 0xF8);
        assert_eq!(JP_HL, 0xE9);
        assert_eq!(RST_38, 0xFF);
        assert_eq!(PREFIX, 0xCB);
        assert_eq!(ILLEGAL_D3, 0xD3);
    }

    #[test]
    fn prefixed_opcodes_are_their_encoding() {
        assert_eq!(RLC_B.to_le_bytes(), [0xCB, 0x00]);
        assert_eq!(BIT_0_HL_PTR.to_le_bytes(), [0xCB, 0x46]);
        assert_eq!(SET_7_A.to_le_bytes(), [0xCB, 0xFF]);
    }

    #[test]
    fn instruction_lens() {
        assert_eq!(instruction_len(NOP), 1);
        assert_eq!(instruction_len(LD_A_N8), 2);
        assert_eq!(instruction_len(STOP_N8), 2);
        assert_eq!(instruction_len(PREFIX), 2);
        assert_eq!(instruction_len(LD_HL_SP_E8), 2);
        assert_eq!(instruction_len(JP_A16), 3);
        assert_eq!(instruction_len(LD_A16_PTR_A), 3);
    }

    #[test]
    fn unconditional_cycles() {
        assert_eq!(cycles(NOP), Cycles { min: 1, max: 1 });
        assert_eq!(cycles(LD_HL_PTR_N8), Cycles { min: 3, max: 3 });
        assert_eq!(cycles(CALL_A16), Cycles { min: 6, max: 6 });
    }

    #[test]
    fn conditional_cycles() {
        assert_eq!(cycles(JR_NZ_E8), Cycles { min: 2, max: 3 });
        assert_eq!(cycles(JP_C_A16), Cycles { min: 3, max: 4 });
        assert_eq!(cycles(CALL_Z_A16), Cycles { min: 3, max: 6 });
        assert_eq!(cycles(RET_NC), Cycles { min: 2, max: 5 });
    }

    #[test]
    fn prefixed_instruction_cycles() {
        assert_eq!(prefixed_cycles(RLC_B), Cycles { min: 2, max: 2 });
        assert_eq!(prefixed_cycles(BIT_0_HL_PTR), Cycles { min: 3, max: 3 });
        assert_eq!(prefixed_cycles(SET_7_HL_PTR), Cycles { min: 4, max: 4 });
    }
}
