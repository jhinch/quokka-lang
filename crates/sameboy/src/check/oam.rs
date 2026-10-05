//! Detects instructions that trigger the DMG/MGB OAM corruption bug.

use super::{Checker, Context, ReadKind};
use quokka_lang_opcodes::*;

/// Reports every instruction that would corrupt OAM on a DMG or MGB if it ran while the PPU
/// was scanning OAM (mode 2).
///
/// The CPU's 16-bit increment/decrement unit drives the value it steps onto the address bus,
/// so stepping a register holding `$FE00-$FEFF` looks like an OAM access to the PPU. This
/// checker flags the hazard rather than the corruption: it ignores both the PPU mode and the
/// model, so a program that only escapes corruption by when it happens to run, or by running
/// on a CGB, still fails.
///
/// The violation kinds are:
///
/// * `inc-dec`: `INC rr` / `DEC rr` with the register in OAM.
/// * `ld-sp-hl`: `LD SP, HL` with `HL` in OAM.
/// * `hl-step`: `LD A, [HL+]` / `LD A, [HL-]` / `LD [HL+], A` / `LD [HL-], A` with `HL` in OAM.
/// * `stack`: `PUSH`, `POP`, `CALL`, `RET`, `RETI`, `RST` or an interrupt dispatch moving `SP`
///   through OAM. Conditional calls and returns only count when taken.
/// * `pc`: an instruction byte fetched from OAM, or a `JR` stepping `PC` into it.
#[derive(Debug, Default)]
pub struct OamCorruptionHazard {
    /// Whether an opcode has been read that the execution callback has not yet confirmed. A
    /// write in this state belongs to an interrupt dispatch, which reads (and discards) the
    /// opcode at `PC` and then pushes `PC` without executing an instruction.
    awaiting_exec: bool,
}

impl OamCorruptionHazard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Whether an address on the bus is seen by the PPU as an OAM access.
const fn is_oam(addr: u16) -> bool {
    matches!(addr, 0xFE00..=0xFEFF)
}

/// The first OAM address a push with the stack pointer at `sp` puts on the bus: `SP` and
/// `SP - 1` as they are decremented, and `SP - 2` for the second write.
fn push_hazard(sp: u16) -> Option<u16> {
    (0..=2)
        .map(|step| sp.wrapping_sub(step))
        .find(|&a| is_oam(a))
}

/// The first OAM address a pop with the stack pointer at `sp` puts on the bus: `SP` and
/// `SP + 1` as they are read and incremented.
fn pop_hazard(sp: u16) -> Option<u16> {
    (0..=1)
        .map(|step| sp.wrapping_add(step))
        .find(|&a| is_oam(a))
}

impl Checker for OamCorruptionHazard {
    fn name(&self) -> &'static str {
        "oam-corruption"
    }

    fn on_exec(&mut self, cx: &mut Context<'_>, pc: u16, opcode: u8) {
        self.awaiting_exec = false;
        let registers = cx.registers();
        let sp = registers.sp;
        let hl = registers.hl;
        let [flags, _] = registers.af.to_le_bytes();

        match opcode {
            INC_BC | INC_DE | INC_HL | INC_SP | DEC_BC | DEC_DE | DEC_HL | DEC_SP => {
                let (name, value) = match opcode {
                    INC_BC | DEC_BC => ("BC", registers.bc),
                    INC_DE | DEC_DE => ("DE", registers.de),
                    INC_HL | DEC_HL => ("HL", hl),
                    _ => ("SP", sp),
                };
                let mnemonic = match opcode {
                    INC_BC | INC_DE | INC_HL | INC_SP => "INC",
                    _ => "DEC",
                };
                if is_oam(value) {
                    cx.report(
                        "inc-dec",
                        Some(value),
                        format!("{mnemonic} {name} with {name}=${value:04X}"),
                    );
                }
            }
            LD_HLI_PTR_A | LD_A_HLI_PTR | LD_HLD_PTR_A | LD_A_HLD_PTR if is_oam(hl) => {
                let mnemonic = match opcode {
                    LD_HLI_PTR_A => "LD [HL+], A",
                    LD_A_HLI_PTR => "LD A, [HL+]",
                    LD_HLD_PTR_A => "LD [HL-], A",
                    _ => "LD A, [HL-]",
                };
                cx.report("hl-step", Some(hl), format!("{mnemonic} with HL=${hl:04X}"));
            }
            LD_SP_HL if is_oam(hl) => {
                cx.report("ld-sp-hl", Some(hl), format!("LD SP, HL with HL=${hl:04X}"));
            }
            // JR e8 puts the address after its operand on the bus before adding the offset. Any
            // later address in OAM has already had the operand fetched from OAM.
            JR_E8 if pc.wrapping_add(2) == 0xFE00 => {
                cx.report("pc", Some(0xFE00), "JR with PC stepping into OAM");
            }
            // a conditional call or return only touches the stack when it is taken
            _ if condition(opcode).is_none_or(|condition| condition.holds(flags)) => {
                let hazard = match stack_effect(opcode) {
                    Some(StackEffect::Push) => push_hazard(sp),
                    Some(StackEffect::Pop) => pop_hazard(sp),
                    None => None,
                };
                if let Some(address) = hazard {
                    cx.report(
                        "stack",
                        Some(address),
                        format!("{} with SP=${sp:04X}", mnemonic(opcode)),
                    );
                }
            }
            _ => {}
        }
    }

    fn on_read(&mut self, cx: &mut Context<'_>, addr: u16, _value: u8, kind: ReadKind) {
        match kind {
            ReadKind::Opcode => {
                self.awaiting_exec = true;
                if is_oam(addr) {
                    cx.report("pc", Some(addr), "opcode fetched from OAM");
                }
            }
            // an instruction starting in OAM was already reported at its opcode, so this is one
            // that starts just below OAM and runs into it
            ReadKind::Operand if addr == 0xFE00 => {
                cx.report("pc", Some(addr), "operand fetched from OAM");
            }
            ReadKind::Operand | ReadKind::Data => {}
        }
    }

    fn on_write(&mut self, cx: &mut Context<'_>, addr: u16, _value: u8) {
        if !self.awaiting_exec {
            return;
        }
        // the first write of an interrupt dispatch, to the decremented stack pointer
        self.awaiting_exec = false;
        let sp = addr.wrapping_add(1);
        if let Some(address) = push_hazard(sp) {
            cx.report(
                "stack",
                Some(address),
                format!("interrupt dispatch with SP=${sp:04X}"),
            );
        }
    }
}
