//! Post-register-allocation peephole rewrites.
//!
//! These run once every virtual register has its final color, and only make
//! sense at that point: each rewrite depends on which physical register an
//! operand landed in.

use crate::lir::{CodeSectionData, Instr, R8Id, VirtualReg, ZeroId};
use crate::reg::Color;
use indexmap::IndexSet;

pub(super) fn cleanup_redundant_instructions(code: &mut CodeSectionData) {
    for (block_id, block) in code.blocks.iter_mut() {
        for instruction in std::mem::take(&mut block.instructions) {
            let retain = match &instruction {
                Instr::Ld8 { target, source }
                    if code.r8_registers[*target].color == code.r8_registers[*source].color =>
                {
                    false
                }
                Instr::Nop
                | Instr::Join { .. }
                | Instr::Split { .. }
                | Instr::MoveZero { .. }
                | Instr::MoveCarry { .. }
                | Instr::LdReturn8 { .. }
                | Instr::LdReturn16 { .. } => false,
                _ => true,
            };
            if retain {
                let index = block.instructions.len();
                for reg in instruction.register_new_defs() {
                    // unfortunately this cannot use code.register_data_mut() as it conflicts
                    // with code.blocks.iter_mut() above
                    match reg {
                        VirtualReg::R8(reg) => &mut code.r8_registers[reg],
                        VirtualReg::R16(reg) => &mut code.r16_registers[reg],
                        VirtualReg::Zero(reg) => &mut code.zero_registers[reg],
                        VirtualReg::Carry(reg) => &mut code.carry_registers[reg],
                        VirtualReg::Call(_) => {
                            unimplemented!("VirtualReg::Call is never a real instruction operand")
                        }
                    }
                    .defined
                    .insert(block_id, index);
                }
                block.instructions.push(instruction);
            } else {
                for reg in instruction.register_new_defs() {
                    // we don't want to remove the register as we are not updating the
                    // uses of the register. Instead, we just clean up the definition pointer
                    match reg {
                        VirtualReg::R8(reg) => &mut code.r8_registers[reg],
                        VirtualReg::R16(reg) => &mut code.r16_registers[reg],
                        VirtualReg::Zero(reg) => &mut code.zero_registers[reg],
                        VirtualReg::Carry(reg) => &mut code.carry_registers[reg],
                        VirtualReg::Call(_) => {
                            unimplemented!("VirtualReg::Call is never a real instruction operand")
                        }
                    }
                    .defined
                    .remove(block_id);
                }
            }
        }
    }
}

/// Rewrites CB-prefixed rotates whose `target`/`source` were colored to `A` into
/// the 1-byte accumulator forms (`RLA`/`RLCA`/`RRA`/`RRCA`).
///
/// The accumulator forms are a byte shorter and idiomatic, but they hard-clear
/// `Z` instead of setting it to `result == 0`, so this only fires when the
/// rotate's `z` output is dead (nothing reads it, e.g. via a following
/// `jumpif z` / `RetIf` / `JpIf`). The `z` register is kept on the rewritten
/// instruction — it still models the write to `Fz` for interference.
pub(super) fn specialize_accumulator_rotates(code: &mut CodeSectionData) {
    let mut used_zero: IndexSet<ZeroId> = IndexSet::new();
    for (_, block) in code.blocks.iter() {
        for instruction in &block.instructions {
            for reg in instruction.register_uses() {
                if let VirtualReg::Zero(z) = reg {
                    used_zero.insert(z);
                }
            }
        }
        for reg in block.terminator.register_uses() {
            if let VirtualReg::Zero(z) = reg {
                used_zero.insert(z);
            }
        }
    }

    for (_, block) in code.blocks.iter_mut() {
        for instruction in &mut block.instructions {
            let is_a = |r8: R8Id| code.r8_registers[r8].color == Some(Color::A);
            match instruction {
                Instr::Rl { z, c, source, c_in } if !used_zero.contains(z) && is_a(*source) => {
                    *instruction = Instr::Rla {
                        z: *z,
                        c: *c,
                        source: source.downcast_to_a_without_move(),
                        c_in: *c_in,
                    };
                }
                Instr::Rlc { z, c, source } if !used_zero.contains(z) && is_a(*source) => {
                    *instruction = Instr::Rlca {
                        z: *z,
                        c: *c,
                        source: source.downcast_to_a_without_move(),
                    };
                }
                Instr::Rr { z, c, source, c_in } if !used_zero.contains(z) && is_a(*source) => {
                    *instruction = Instr::Rra {
                        z: *z,
                        c: *c,
                        source: source.downcast_to_a_without_move(),
                        c_in: *c_in,
                    };
                }
                Instr::Rrc { z, c, source } if !used_zero.contains(z) && is_a(*source) => {
                    *instruction = Instr::Rrca {
                        z: *z,
                        c: *c,
                        source: source.downcast_to_a_without_move(),
                    };
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Identifier;
    use crate::lir::{Block, BlockId, FlagId, Register, Terminator};
    use slotmap::{SlotMap, SparseSecondaryMap};

    /// Builds a one-block function whose sole instruction is `RLC target` (with
    /// `source == target`), coloring `target`/`source` with `rotate_color`. The
    /// block's terminator is a bare `RET`; callers that want the rotate's `z`
    /// flag to be live overwrite it.
    fn code_with_single_rlc(rotate_color: Option<Color>) -> (CodeSectionData, BlockId, ZeroId) {
        let mut code = CodeSectionData {
            region: None,
            addr: None,
            functions: SparseSecondaryMap::new(),
            blocks: SlotMap::with_key(),
            zero_registers: SlotMap::with_key(),
            carry_registers: SlotMap::with_key(),
            r8_registers: SlotMap::with_key(),
            r16_registers: SlotMap::with_key(),
        };
        let r8 = code.r8_registers.insert(Register {
            color: rotate_color,
            ..Register::default()
        });
        let z = code.zero_registers.insert(Register {
            color: Some(Color::Fz),
            ..Register::default()
        });
        let c = code.carry_registers.insert(Register {
            color: Some(Color::Fc),
            ..Register::default()
        });
        let block = code.blocks.insert(Block {
            instructions: vec![Instr::Rlc { z, c, source: r8 }],
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        code.functions.insert(block, Identifier::from("f"));
        code.r8_registers[r8].defined.insert(block, 0);
        code.zero_registers[z].defined.insert(block, 0);
        code.carry_registers[c].defined.insert(block, 0);
        (code, block, z)
    }

    #[test]
    fn specialize_accumulator_rotates_rewrites_dead_zero_a_rotate() {
        let (mut code, block, z) = code_with_single_rlc(Some(Color::A));
        specialize_accumulator_rotates(&mut code);
        assert!(
            matches!(
                code.blocks[block].instructions[0],
                Instr::Rlca { z: rewritten_z, .. } if rewritten_z == z
            ),
            "an A-colored RLC with a dead zero should become RLCA, keeping its z, got {:?}",
            code.blocks[block].instructions[0]
        );
        assert!(
            code.zero_registers[z].defined.contains_key(block),
            "the retained zero flag still models the write to Fz and keeps its definition"
        );
    }

    #[test]
    fn specialize_accumulator_rotates_keeps_cb_form_when_zero_is_live() {
        let (mut code, block, z) = code_with_single_rlc(Some(Color::A));
        code.blocks[block].terminator = Terminator::RetIf {
            source: FlagId::Zero(z),
            negate: false,
            else_block: block,
        };
        specialize_accumulator_rotates(&mut code);
        assert!(
            matches!(code.blocks[block].instructions[0], Instr::Rlc { .. }),
            "an RLC whose zero flag feeds a RetIf must keep the CB form, got {:?}",
            code.blocks[block].instructions[0]
        );
    }

    #[test]
    fn specialize_accumulator_rotates_keeps_cb_form_for_non_a_register() {
        let (mut code, block, _z) = code_with_single_rlc(Some(Color::B));
        specialize_accumulator_rotates(&mut code);
        assert!(
            matches!(code.blocks[block].instructions[0], Instr::Rlc { .. }),
            "a non-A-colored RLC must keep the CB form, got {:?}",
            code.blocks[block].instructions[0]
        );
    }
}
