//! The checker interface: independent units that observe a running emulator through
//! execution and memory events and record [`Violation`]s.

mod halt;
mod oam;
mod ppu;

pub use halt::HaltHazard;
pub use oam::OamCorruptionHazard;
pub use ppu::PpuBlockedAccess;

use crate::hooks;
use quokka_lang_sameboy_sys::GB_gameboy_t;
use std::fmt;

/// A runtime check that observes the emulator as it runs.
///
/// Every entry point has an empty default, so a checker only implements the events it cares
/// about. Checkers get read-only access to the emulator through [`Context`], and record what
/// they find with [`Context::report`].
///
/// A checker that panics does not unwind through SameBoy: the panic is caught at the callback
/// boundary and resumed from [`Sameboy::run`](crate::Sameboy::run) once the instruction is
/// complete.
pub trait Checker {
    /// A short, stable name identifying this checker in the violations it reports.
    fn name(&self) -> &'static str;

    /// Called once the opcode at `pc` has been fetched, before the instruction executes. The
    /// registers in `cx` hold their pre-execution values.
    fn on_exec(&mut self, _cx: &mut Context<'_>, _pc: u16, _opcode: u8) {}

    /// Called for every read the CPU makes from the bus, with the `value` it read.
    fn on_read(&mut self, _cx: &mut Context<'_>, _addr: u16, _value: u8, _kind: ReadKind) {}

    /// Called for every write the CPU makes to the bus, before `value` is stored.
    fn on_write(&mut self, _cx: &mut Context<'_>, _addr: u16, _value: u8) {}

    /// Called once the instruction at `pc` has finished executing, with the registers and
    /// emulator state in `cx` as it left them. A step that executes no instruction, because
    /// the CPU is halted or an interrupt is dispatched instead, does not get one.
    fn on_exec_end(&mut self, _cx: &mut Context<'_>, _pc: u16, _opcode: u8) {}

    /// Called by [`Sameboy::finish`](crate::Sameboy::finish) once the run is over, for checks
    /// that can only be made against the final state.
    fn on_finish(&mut self, _cx: &mut Context<'_>) {}
}

/// What a read from the bus was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// The first byte of an instruction. An interrupt dispatch also makes (and discards) one
    /// of these at the interrupted `PC`.
    Opcode,
    /// A byte following the opcode that the CPU fetches at `PC` as part of decoding: immediate
    /// operands, the second byte of a `CB`-prefixed instruction, and the look-ahead reads
    /// `HALT` and `STOP` make of the byte after them.
    Operand,
    /// A read the instruction itself performs, such as `LD A, [HL]` or `POP BC`.
    Data,
}

/// Something a [`Checker`] found wrong with the running program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The [`Checker::name`] of the checker that reported it.
    pub checker: &'static str,
    /// What went wrong, as a short, stable identifier chosen by the checker.
    pub kind: &'static str,
    /// The address of the instruction that was executing.
    pub pc: u16,
    /// The memory address involved, if any.
    pub address: Option<u16>,
    /// A human-readable description.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{} at PC=${:04X}", self.checker, self.kind, self.pc)?;
        if let Some(address) = self.address {
            write!(f, ", address ${address:04X}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// A snapshot of the CPU registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Registers {
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub sp: u16,
    pub pc: u16,
}

impl fmt::Display for Registers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X}",
            self.af, self.bc, self.de, self.hl, self.sp
        )
    }
}

/// A [`Checker`]'s read-only view of the emulator during an event.
pub struct Context<'a> {
    gb: *mut GB_gameboy_t,
    pc: u16,
    checker: &'static str,
    violations: &'a mut Vec<Violation>,
}

impl<'a> Context<'a> {
    /// # Safety
    ///
    /// `gb` must point to an initialized `GB_gameboy_t` that stays valid, and is not accessed
    /// through any Rust reference, for as long as the returned `Context` is alive.
    pub(crate) unsafe fn new(
        gb: *mut GB_gameboy_t,
        pc: u16,
        checker: &'static str,
        violations: &'a mut Vec<Violation>,
    ) -> Self {
        Context {
            gb,
            pc,
            checker,
            violations,
        }
    }

    /// The address of the instruction being executed.
    #[must_use]
    pub fn pc(&self) -> u16 {
        self.pc
    }

    /// The current CPU registers. `pc` is the address of the instruction being executed
    /// rather than the CPU's live program counter, which is already part-way through it.
    #[must_use]
    pub fn registers(&self) -> Registers {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        let mut registers = unsafe { hooks::registers(self.gb) };
        registers.pc = self.pc;
        registers
    }

    /// Read a byte of memory without side effects: no checker sees it as a read, and it does
    /// not disturb the emulated hardware.
    #[must_use]
    pub fn peek(&self, addr: u16) -> u8 {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::peek(self.gb, addr) }
    }

    /// Whether interrupts are enabled (the interrupt master enable flag).
    #[must_use]
    pub fn ime(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::ime(self.gb) }
    }

    /// Whether the CPU is halted, waiting for an interrupt.
    #[must_use]
    pub fn halted(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::halted(self.gb) }
    }

    /// Whether the last `HALT` triggered the HALT bug, so the next opcode byte will be
    /// executed twice.
    #[must_use]
    pub fn halt_bug(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::halt_bug(self.gb) }
    }

    /// Whether the emulated model is a Game Boy Color (or later).
    #[must_use]
    pub fn is_cgb(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::is_cgb(self.gb) }
    }

    /// Whether the PPU is currently blocking CPU reads of OAM.
    #[must_use]
    pub fn oam_read_blocked(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::oam_read_blocked(self.gb) }
    }

    /// Whether the PPU is currently blocking CPU writes to OAM.
    #[must_use]
    pub fn oam_write_blocked(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::oam_write_blocked(self.gb) }
    }

    /// Whether the PPU is currently blocking CPU reads of VRAM.
    #[must_use]
    pub fn vram_read_blocked(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::vram_read_blocked(self.gb) }
    }

    /// Whether the PPU is currently blocking CPU writes to VRAM.
    #[must_use]
    pub fn vram_write_blocked(&self) -> bool {
        // SAFETY: `Context::new` guarantees `gb` is valid and not otherwise borrowed
        unsafe { hooks::vram_write_blocked(self.gb) }
    }

    /// Record a violation against the instruction being executed.
    pub fn report(&mut self, kind: &'static str, address: Option<u16>, detail: impl Into<String>) {
        self.violations.push(Violation {
            checker: self.checker,
            kind,
            pc: self.pc,
            address,
            detail: detail.into(),
        });
    }
}
