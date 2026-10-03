//! Detects instructions that trigger the DMG/MGB OAM corruption bug.

use super::{Checker, Context, ReadKind};

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

const ZERO_FLAG: u16 = 0x80;
const CARRY_FLAG: u16 = 0x10;

/// Whether an address on the bus is seen by the PPU as an OAM access.
const fn is_oam(addr: u16) -> bool {
    matches!(addr, 0xFE00..=0xFEFF)
}

/// Whether the condition encoded in a `JR cc` / `CALL cc` / `RET cc` opcode holds for `af`.
const fn condition_holds(opcode: u8, af: u16) -> bool {
    match (opcode >> 3) & 0x3 {
        0 => af & ZERO_FLAG == 0,
        1 => af & ZERO_FLAG != 0,
        2 => af & CARRY_FLAG == 0,
        _ => af & CARRY_FLAG != 0,
    }
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

        let push = |cx: &mut Context<'_>, mnemonic: &str| {
            if let Some(address) = push_hazard(sp) {
                cx.report(
                    "stack",
                    Some(address),
                    format!("{mnemonic} with SP=${sp:04X}"),
                );
            }
        };
        let pop = |cx: &mut Context<'_>, mnemonic: &str| {
            if let Some(address) = pop_hazard(sp) {
                cx.report(
                    "stack",
                    Some(address),
                    format!("{mnemonic} with SP=${sp:04X}"),
                );
            }
        };

        match opcode {
            // INC rr / DEC rr
            0x03 | 0x13 | 0x23 | 0x33 | 0x0B | 0x1B | 0x2B | 0x3B => {
                let (name, value) = match opcode >> 4 {
                    0 => ("BC", registers.bc),
                    1 => ("DE", registers.de),
                    2 => ("HL", hl),
                    _ => ("SP", sp),
                };
                let mnemonic = if opcode & 0x08 == 0 { "INC" } else { "DEC" };
                if is_oam(value) {
                    cx.report(
                        "inc-dec",
                        Some(value),
                        format!("{mnemonic} {name} with {name}=${value:04X}"),
                    );
                }
            }
            // LD [HL+], A / LD A, [HL+] / LD [HL-], A / LD A, [HL-]
            0x22 | 0x2A | 0x32 | 0x3A if is_oam(hl) => {
                let mnemonic = match opcode {
                    0x22 => "LD [HL+], A",
                    0x2A => "LD A, [HL+]",
                    0x32 => "LD [HL-], A",
                    _ => "LD A, [HL-]",
                };
                cx.report("hl-step", Some(hl), format!("{mnemonic} with HL=${hl:04X}"));
            }
            // LD SP, HL
            0xF9 if is_oam(hl) => {
                cx.report("ld-sp-hl", Some(hl), format!("LD SP, HL with HL=${hl:04X}"));
            }
            // PUSH rr
            0xC5 | 0xD5 | 0xE5 | 0xF5 => push(cx, "PUSH"),
            // CALL a16
            0xCD => push(cx, "CALL"),
            // CALL cc, a16
            0xC4 | 0xCC | 0xD4 | 0xDC if condition_holds(opcode, registers.af) => {
                push(cx, "CALL");
            }
            // RST
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => push(cx, "RST"),
            // POP rr
            0xC1 | 0xD1 | 0xE1 | 0xF1 => pop(cx, "POP"),
            // RET
            0xC9 => pop(cx, "RET"),
            // RETI
            0xD9 => pop(cx, "RETI"),
            // RET cc
            0xC0 | 0xC8 | 0xD0 | 0xD8 if condition_holds(opcode, registers.af) => {
                pop(cx, "RET");
            }
            // JR r8 puts the address after its operand on the bus before adding the offset. Any
            // later address in OAM has already had the operand fetched from OAM.
            0x18 if pc.wrapping_add(2) == 0xFE00 => {
                cx.report("pc", Some(0xFE00), "JR with PC stepping into OAM");
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
