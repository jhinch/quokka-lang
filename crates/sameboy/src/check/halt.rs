//! Detects `HALT` instructions that trigger the HALT bug or that can never wake up.

use super::{Checker, Context};

/// Reports every `HALT` that misbehaves because of the interrupt state it is executed in.
///
/// The violation kinds are:
///
/// * `bug`: `HALT` with interrupts disabled (IME = 0) and an enabled interrupt already pending
///   (`IE & IF & $1F != 0`). The CPU does not halt and fails to advance `PC`, so the byte after
///   the `HALT` is executed twice. This happens on every model.
/// * `hang`: `HALT` with no interrupt enabled (`IE & $1F == 0`). Nothing can wake the CPU, so
///   the program is stuck for good, whatever IME is.
///
/// Whether the bug was triggered is the emulator's own verdict, so a `HALT` that only behaves
/// because of when an interrupt happens to arrive is not reported.
#[derive(Debug, Default)]
pub struct HaltHazard;

impl HaltHazard {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

const HALT: u8 = 0x76;
const INTERRUPT_FLAGS: u16 = 0xFF0F;
const INTERRUPT_ENABLE: u16 = 0xFFFF;
/// The bits of `IE` and `IF` that correspond to an interrupt.
const INTERRUPT_MASK: u8 = 0x1F;

impl Checker for HaltHazard {
    fn name(&self) -> &'static str {
        "halt"
    }

    fn on_exec_end(&mut self, cx: &mut Context<'_>, pc: u16, opcode: u8) {
        if opcode != HALT {
            return;
        }
        // nothing has run since the `HALT` made its decision, so these are the values it saw
        let enabled = cx.peek(INTERRUPT_ENABLE) & INTERRUPT_MASK;
        if cx.halt_bug() {
            let flags = cx.peek(INTERRUPT_FLAGS) & INTERRUPT_MASK;
            cx.report(
                "bug",
                Some(pc.wrapping_add(1)),
                format!(
                    "HALT with IME=0, IE=${enabled:02X} and IF=${flags:02X}: the next byte is \
                     executed twice"
                ),
            );
        } else if enabled == 0 {
            cx.report(
                "hang",
                None,
                "HALT with no interrupt enabled in IE: nothing can wake the CPU",
            );
        }
    }
}
