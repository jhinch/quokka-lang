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

/// The condition a conditional jump, call or return tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Condition {
    /// `NZ`
    NotZero,
    /// `Z`
    Zero,
    /// `NC`
    NoCarry,
    /// `C`
    Carry,
}

/// The zero flag's bit in the `F` register.
pub const ZERO_FLAG: u8 = 0x80;
/// The carry flag's bit in the `F` register.
pub const CARRY_FLAG: u8 = 0x10;

impl Condition {
    /// Whether the condition holds when the `F` register is `flags`.
    #[must_use]
    pub const fn holds(self, flags: u8) -> bool {
        match self {
            Condition::NotZero => flags & ZERO_FLAG == 0,
            Condition::Zero => flags & ZERO_FLAG != 0,
            Condition::NoCarry => flags & CARRY_FLAG == 0,
            Condition::Carry => flags & CARRY_FLAG != 0,
        }
    }
}

/// What an instruction does to the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackEffect {
    /// Two bytes are written below `SP`, which moves down to them.
    Push,
    /// Two bytes are read at `SP`, which moves up past them.
    Pop,
}

/// The mnemonic of the instruction starting with `opcode`. An opcode which is not an
/// instruction has a mnemonic of its own, such as `ILLEGAL_D3`.
#[must_use]
pub const fn mnemonic(opcode: u8) -> &'static str {
    MNEMONIC[opcode as usize]
}

/// The condition the instruction starting with `opcode` tests, or `None` if it is
/// unconditional.
#[must_use]
pub const fn condition(opcode: u8) -> Option<Condition> {
    match opcode {
        JR_NZ_E8 | JP_NZ_A16 | CALL_NZ_A16 | RET_NZ => Some(Condition::NotZero),
        JR_Z_E8 | JP_Z_A16 | CALL_Z_A16 | RET_Z => Some(Condition::Zero),
        JR_NC_E8 | JP_NC_A16 | CALL_NC_A16 | RET_NC => Some(Condition::NoCarry),
        JR_C_E8 | JP_C_A16 | CALL_C_A16 | RET_C => Some(Condition::Carry),
        _ => None,
    }
}

/// What the instruction starting with `opcode` does to the stack, or `None` if it neither
/// pushes nor pops. A conditional call or return only has its effect when its [`condition`]
/// holds.
///
/// Instructions which move `SP` without reading or writing the stack, such as `INC SP` and
/// `ADD SP, e8`, have no effect.
#[must_use]
pub const fn stack_effect(opcode: u8) -> Option<StackEffect> {
    match opcode {
        PUSH_BC | PUSH_DE | PUSH_HL | PUSH_AF => Some(StackEffect::Push),
        CALL_A16 | CALL_NZ_A16 | CALL_Z_A16 | CALL_NC_A16 | CALL_C_A16 => Some(StackEffect::Push),
        RST_00 | RST_08 | RST_10 | RST_18 | RST_20 | RST_28 | RST_30 | RST_38 => {
            Some(StackEffect::Push)
        }
        POP_BC | POP_DE | POP_HL | POP_AF => Some(StackEffect::Pop),
        RET | RETI | RET_NZ | RET_Z | RET_NC | RET_C => Some(StackEffect::Pop),
        _ => None,
    }
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
    fn mnemonics() {
        assert_eq!(mnemonic(NOP), "NOP");
        assert_eq!(mnemonic(LD_HL_PTR_N8), "LD");
        assert_eq!(mnemonic(RET_NC), "RET");
        assert_eq!(mnemonic(RST_38), "RST");
        assert_eq!(mnemonic(ILLEGAL_D3), "ILLEGAL_D3");
    }

    /// The opcodes whose mnemonic is `mnemonic`.
    fn with_mnemonic(name: &str) -> impl Iterator<Item = u8> {
        (0..=u8::MAX).filter(move |&opcode| mnemonic(opcode) == name)
    }

    #[test]
    fn conditions() {
        assert_eq!(condition(JR_NZ_E8), Some(Condition::NotZero));
        assert_eq!(condition(JP_Z_A16), Some(Condition::Zero));
        assert_eq!(condition(CALL_NC_A16), Some(Condition::NoCarry));
        assert_eq!(condition(RET_C), Some(Condition::Carry));
        for opcode in [
            JR_E8,
            JP_A16,
            JP_HL,
            CALL_A16,
            RET,
            RETI,
            LD_A_C,
            LDH_A_C_PTR,
        ] {
            assert_eq!(condition(opcode), None, "{opcode:#04X}");
        }
    }

    #[test]
    fn only_jumps_calls_and_returns_are_conditional() {
        let conditional = (0..=u8::MAX).filter(|&opcode| condition(opcode).is_some());
        // an unconditional and four conditional forms each, less the unconditional ones,
        // `JP HL` and `RETI`
        assert_eq!(conditional.clone().count(), 16);
        for opcode in conditional {
            assert!(
                matches!(mnemonic(opcode), "JR" | "JP" | "CALL" | "RET"),
                "{opcode:#04X}"
            );
            let Cycles { min, max } = cycles(opcode);
            assert!(min < max, "{opcode:#04X}");
        }
    }

    #[test]
    fn conditions_hold_by_flag() {
        assert!(Condition::Zero.holds(ZERO_FLAG));
        assert!(!Condition::Zero.holds(CARRY_FLAG));
        assert!(Condition::NotZero.holds(CARRY_FLAG));
        assert!(!Condition::NotZero.holds(ZERO_FLAG));
        assert!(Condition::Carry.holds(CARRY_FLAG));
        assert!(!Condition::Carry.holds(ZERO_FLAG));
        assert!(Condition::NoCarry.holds(ZERO_FLAG));
        assert!(!Condition::NoCarry.holds(CARRY_FLAG));
    }

    #[test]
    fn stack_effects() {
        for name in ["PUSH", "CALL", "RST"] {
            for opcode in with_mnemonic(name) {
                assert_eq!(
                    stack_effect(opcode),
                    Some(StackEffect::Push),
                    "{opcode:#04X}"
                );
            }
        }
        for name in ["POP", "RET", "RETI"] {
            for opcode in with_mnemonic(name) {
                assert_eq!(
                    stack_effect(opcode),
                    Some(StackEffect::Pop),
                    "{opcode:#04X}"
                );
            }
        }
        let affecting = (0..=u8::MAX).filter(|&opcode| stack_effect(opcode).is_some());
        assert_eq!(affecting.count(), 4 + 5 + 8 + 4 + 5 + 1);
        for opcode in [
            INC_SP,
            DEC_SP,
            ADD_SP_E8,
            LD_SP_HL,
            LD_SP_N16,
            LD_A16_PTR_SP,
            JP_A16,
        ] {
            assert_eq!(stack_effect(opcode), None, "{opcode:#04X}");
        }
    }

    #[test]
    fn prefixed_instruction_cycles() {
        assert_eq!(prefixed_cycles(RLC_B), Cycles { min: 2, max: 2 });
        assert_eq!(prefixed_cycles(BIT_0_HL_PTR), Cycles { min: 3, max: 3 });
        assert_eq!(prefixed_cycles(SET_7_HL_PTR), Cycles { min: 4, max: 4 });
    }
}
