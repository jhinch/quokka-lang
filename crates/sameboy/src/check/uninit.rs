//! Detects reads of RAM the program has not written.

use super::{Checker, Context, ReadKind};
use quokka_lang_opcodes::*;
use std::collections::BTreeMap;

/// Reports data reads of RAM which holds nothing the program put there, by keeping a shadow
/// of which bytes have been written since the checker was created. The emulator fills such
/// memory with random values, as hardware does, so the program's behavior depends on them.
///
/// What is tracked is decided by the address alone:
///
/// * Work RAM (`$C000-$DFFF`, and through its echo at `$E000-$FDFF`), High RAM
///   (`$FF80-$FFFE`) and OAM (`$FE00-$FE9F`) start out undefined, and a write defines the
///   byte. An OAM DMA transfer gives each byte of OAM the state of the byte copied into it.
/// * Everything else is always defined: ROM, the I/O registers, which are hardware state
///   rather than storage and are naturally read before they are written, video RAM, which the
///   boot ROM initializes, and cartridge RAM, which outlives the program.
///
/// Stack bytes become undefined again once `SP` moves above them: the bytes released by a
/// `POP`, a `RET` or `RETI`, `ADD SP, e` and `INC SP`. Loading `SP` outright (`LD SP, d16`,
/// `LD SP, HL`) switches stacks and releases nothing.
///
/// A byte the [`MemoryMap`](crate::MemoryMap) declares as narrow holds a value smaller than a
/// byte, which a program stores by reading the byte, replacing the value's bits and writing
/// it back, so the first store reads the byte undefined. Such a read is therefore held back
/// until the byte is next accessed: a write forgives it, and it is reported if the byte is
/// read again first, or is never written.
///
/// The violation kinds are:
///
/// * `never-written`: the byte has not been written.
/// * `released`: the byte was on the stack and `SP` has since moved above it.
/// * `narrow`: an undefined narrow byte was read twice, or read and never written.
///
/// A byte is reported once and counts as defined afterwards. Only the reads an instruction
/// performs are checked, not the fetching of the instruction itself. Work RAM banks are not
/// told apart: the switchable bank of a Game Boy Color shares one shadow.
#[derive(Debug)]
pub struct UninitializedRead {
    shadow: Box<[State]>,
    /// The undefined narrow bytes which have been read and not yet accessed again, with the
    /// address of the instruction that read each.
    pending: BTreeMap<u16, u16>,
    /// The value of `SP` before the instruction being executed.
    sp: u16,
}

/// What the program has done with a byte of tracked memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not written since the checker was created.
    NeverWritten,
    /// Written, then released from the stack.
    Released,
    Written,
}

const OAM_START: u16 = 0xFE00;
const OAM_LEN: u16 = 0xA0;
/// The register which starts an OAM DMA transfer from the page written to it.
const DMA: u16 = 0xFF46;

/// The address `addr` is tracked under, or `None` if it is always defined.
const fn tracked(addr: u16) -> Option<u16> {
    match addr {
        0xC000..=0xDFFF | 0xFE00..=0xFE9F | 0xFF80..=0xFFFE => Some(addr),
        // the echo of work RAM
        0xE000..=0xFDFF => Some(addr - 0x2000),
        _ => None,
    }
}

/// How many bytes of stack the instruction starting with `opcode` released, having moved `SP`
/// from `before` to `after`.
const fn released(opcode: u8, before: u16, after: u16) -> u16 {
    let releases = matches!(stack_effect(opcode), Some(StackEffect::Pop))
        || matches!(opcode, INC_SP | ADD_SP_E8);
    let moved = after.wrapping_sub(before);
    // `ADD SP, e8` moves `SP` up by at most 127, and anything larger is a move down
    if releases && moved <= 0x7F { moved } else { 0 }
}

impl Default for UninitializedRead {
    fn default() -> Self {
        Self::new()
    }
}

impl UninitializedRead {
    #[must_use]
    pub fn new() -> Self {
        UninitializedRead {
            shadow: vec![State::NeverWritten; 0x1_0000].into_boxed_slice(),
            pending: BTreeMap::new(),
            sp: 0,
        }
    }

    fn state(&mut self, addr: u16) -> &mut State {
        &mut self.shadow[usize::from(addr)]
    }

    /// Give each byte of OAM the state of the byte a DMA transfer from `page` copies into it.
    fn transfer_to_oam(&mut self, page: u8) {
        // the transfer reads work RAM in place of its echo and everything above it
        let page = if page >= 0xE0 { page - 0x20 } else { page };
        let source = u16::from(page) << 8;
        for offset in 0..OAM_LEN {
            let state = match tracked(source + offset) {
                Some(addr) => *self.state(addr),
                None => State::Written,
            };
            *self.state(OAM_START + offset) = state;
            self.pending.remove(&(OAM_START + offset));
        }
    }
}

impl Checker for UninitializedRead {
    fn name(&self) -> &'static str {
        "uninitialized-read"
    }

    fn on_exec(&mut self, cx: &mut Context<'_>, _pc: u16, _opcode: u8) {
        self.sp = cx.registers().sp;
    }

    fn on_read(&mut self, cx: &mut Context<'_>, addr: u16, _value: u8, kind: ReadKind) {
        if kind != ReadKind::Data {
            return;
        }
        let Some(tracked) = tracked(addr) else {
            return;
        };
        let state = std::mem::replace(self.state(tracked), State::Written);
        if let Some(first) = self.pending.remove(&tracked) {
            cx.report(
                "narrow",
                Some(addr),
                format!(
                    "second read of a narrow byte which has not been written, first read at \
                     PC=${first:04X}"
                ),
            );
            return;
        }
        match state {
            State::Written => {}
            State::NeverWritten
                if cx
                    .memory_map()
                    .is_some_and(|memory_map| memory_map.is_narrow(tracked)) =>
            {
                // most likely the read half of a store, so wait and see
                *self.state(tracked) = State::NeverWritten;
                self.pending.insert(tracked, cx.pc());
            }
            State::NeverWritten => cx.report(
                "never-written",
                Some(addr),
                "read of memory which has not been written",
            ),
            State::Released => cx.report(
                "released",
                Some(addr),
                "read of stack memory released when SP moved above it",
            ),
        }
    }

    fn on_write(&mut self, _cx: &mut Context<'_>, addr: u16, value: u8) {
        if addr == DMA {
            self.transfer_to_oam(value);
        }
        if let Some(tracked) = tracked(addr) {
            *self.state(tracked) = State::Written;
            self.pending.remove(&tracked);
        }
    }

    fn on_exec_end(&mut self, cx: &mut Context<'_>, _pc: u16, opcode: u8) {
        for offset in 0..released(opcode, self.sp, cx.registers().sp) {
            if let Some(tracked) = tracked(self.sp.wrapping_add(offset)) {
                *self.state(tracked) = State::Released;
                self.pending.remove(&tracked);
            }
        }
    }

    fn on_finish(&mut self, cx: &mut Context<'_>) {
        for (addr, first) in std::mem::take(&mut self.pending) {
            *self.state(addr) = State::Written;
            cx.report(
                "narrow",
                Some(addr),
                format!("narrow byte read at PC=${first:04X} was never written"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_and_oam_are_tracked_and_the_echo_shares_work_ram() {
        assert_eq!(tracked(0xBFFF), None);
        assert_eq!(tracked(0xC000), Some(0xC000));
        assert_eq!(tracked(0xDFFF), Some(0xDFFF));
        assert_eq!(tracked(0xE000), Some(0xC000));
        assert_eq!(tracked(0xFDFF), Some(0xDDFF));
        assert_eq!(tracked(0xFE00), Some(0xFE00));
        assert_eq!(tracked(0xFE9F), Some(0xFE9F));
        assert_eq!(tracked(0xFEA0), None);
        assert_eq!(tracked(0xFF80), Some(0xFF80));
        assert_eq!(tracked(0xFFFE), Some(0xFFFE));
    }

    #[test]
    fn rom_video_ram_cartridge_ram_and_io_are_always_defined() {
        for addr in [
            0x0000, 0x7FFF, 0x8000, 0x9FFF, 0xA000, 0xFF00, 0xFF7F, 0xFFFF,
        ] {
            assert_eq!(tracked(addr), None, "${addr:04X}");
        }
    }

    #[test]
    fn only_instructions_which_pop_release_stack() {
        assert_eq!(released(POP_BC, 0xFFF0, 0xFFF2), 2);
        assert_eq!(released(RET, 0xFFF0, 0xFFF2), 2);
        assert_eq!(released(INC_SP, 0xFFF0, 0xFFF1), 1);
        // not taken
        assert_eq!(released(RET_Z, 0xFFF0, 0xFFF0), 0);
        // ADD SP, 4 and ADD SP, -4
        assert_eq!(released(ADD_SP_E8, 0xFFF0, 0xFFF4), 4);
        assert_eq!(released(ADD_SP_E8, 0xFFF0, 0xFFEC), 0);
        // these switch stacks
        assert_eq!(released(LD_SP_N16, 0xFFF0, 0xFFF8), 0);
        assert_eq!(released(LD_SP_HL, 0xFFF0, 0xFFF8), 0);
        assert_eq!(released(PUSH_BC, 0xFFF0, 0xFFEE), 0);
    }

    #[test]
    fn dma_copies_the_state_of_its_source() {
        let mut checker = UninitializedRead::new();
        *checker.state(0xC101) = State::Written;
        checker.transfer_to_oam(0xC1);
        assert_eq!(*checker.state(0xFE00), State::NeverWritten);
        assert_eq!(*checker.state(0xFE01), State::Written);

        // from the echo of the same page
        *checker.state(0xC102) = State::Written;
        checker.transfer_to_oam(0xE1);
        assert_eq!(*checker.state(0xFE02), State::Written);

        // ROM is always defined
        checker.transfer_to_oam(0x40);
        assert_eq!(*checker.state(0xFE00), State::Written);
        assert_eq!(*checker.state(0xFE9F), State::Written);
    }
}
