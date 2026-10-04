//! The SameBoy callbacks and the Rust-side state they feed, reached through SameBoy's user
//! data pointer.

use crate::check::{Checker, Context, ReadKind, Registers, Violation};
use crate::trace::Trace;
use quokka_lang_sameboy_sys::*;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The state behind SameBoy's user data pointer.
///
/// While `GB_run` is executing, the callbacks are the only code that touches this, each through
/// a `&mut` that lasts for one callback. [`Sameboy`](crate::Sameboy) therefore holds it as a
/// raw pointer and only dereferences it between runs.
pub(crate) struct Hooks {
    pub(crate) checkers: Vec<Box<dyn Checker>>,
    pub(crate) violations: Vec<Violation>,
    pub(crate) trace: Trace,
    /// The address of the instruction being executed.
    pc: u16,
    /// The opcode of the instruction being executed.
    opcode: u8,
    /// Whether the current step has fetched its opcode yet.
    fetched: bool,
    /// How many of the reads following the opcode are still part of fetching the instruction.
    fetch_reads_remaining: u8,
    /// How many of those are operand bytes belonging to the instruction's encoding.
    operand_bytes_remaining: u8,
    /// A panic caught in a callback, waiting to be resumed once `GB_run` returns.
    panic: Option<Box<dyn Any + Send>>,
}

impl Hooks {
    pub(crate) fn new() -> Self {
        Hooks {
            checkers: Vec::new(),
            violations: Vec::new(),
            trace: Trace::new(Trace::DEFAULT_CAPACITY),
            pc: 0,
            opcode: 0,
            fetched: false,
            fetch_reads_remaining: 0,
            operand_bytes_remaining: 0,
            panic: None,
        }
    }

    /// Prepare for the next `GB_run`, which executes at most one instruction.
    pub(crate) fn begin_step(&mut self) {
        self.fetched = false;
        self.fetch_reads_remaining = 0;
        self.operand_bytes_remaining = 0;
    }

    /// Finish the step `GB_run` just made, running every checker's `on_exec_end` if it
    /// executed an instruction.
    ///
    /// # Safety
    ///
    /// Same as [`Hooks::dispatch`].
    pub(crate) unsafe fn end_step(&mut self, gb: *mut GB_gameboy_t) {
        if !self.fetched {
            return;
        }
        let (pc, opcode) = (self.pc, self.opcode);
        // SAFETY: the caller's guarantees are the ones `dispatch` requires
        unsafe { self.dispatch(gb, |checker, cx| checker.on_exec_end(cx, pc, opcode)) };
    }

    /// Take the panic caught during the last step, if a callback panicked.
    pub(crate) fn take_panic(&mut self) -> Option<Box<dyn Any + Send>> {
        self.panic.take()
    }

    /// Forget the execution history, for when the emulator's state is replaced wholesale.
    pub(crate) fn clear_history(&mut self) {
        self.trace.clear();
        self.begin_step();
    }

    /// Run every checker's `on_finish`.
    ///
    /// # Safety
    ///
    /// `gb` must point to an initialized `GB_gameboy_t` that is not accessed through any Rust
    /// reference for the duration of the call.
    pub(crate) unsafe fn finish(&mut self, gb: *mut GB_gameboy_t) {
        // SAFETY: `GB_get_registers` requires an initialized `GB_gameboy_t`, guaranteed by the
        // caller
        self.pc = unsafe { registers(gb) }.pc;
        // SAFETY: the caller's guarantees are the ones `dispatch` requires
        unsafe { self.dispatch(gb, |checker, cx| checker.on_finish(cx)) };
    }

    /// # Safety
    ///
    /// `gb` must point to an initialized `GB_gameboy_t` that is not accessed through any Rust
    /// reference for the duration of the call.
    unsafe fn dispatch(
        &mut self,
        gb: *mut GB_gameboy_t,
        mut event: impl FnMut(&mut dyn Checker, &mut Context<'_>),
    ) {
        for checker in &mut self.checkers {
            // SAFETY: the caller guarantees `gb` is valid and not otherwise borrowed
            let mut cx = unsafe { Context::new(gb, self.pc, checker.name(), &mut self.violations) };
            event(checker.as_mut(), &mut cx);
        }
    }

    /// # Safety
    ///
    /// Same as [`Hooks::dispatch`].
    unsafe fn on_exec(&mut self, gb: *mut GB_gameboy_t, pc: u16, opcode: u8) {
        self.pc = pc;
        self.opcode = opcode;
        self.fetched = true;
        self.fetch_reads_remaining = fetch_reads(opcode);
        self.operand_bytes_remaining = instruction_len(opcode) - 1;

        // SAFETY: `GB_get_registers` requires an initialized `GB_gameboy_t`, guaranteed by the
        // caller
        let mut registers = unsafe { registers(gb) };
        registers.pc = pc;
        self.trace.push(pc, opcode, registers);

        // SAFETY: the caller's guarantees are the ones `dispatch` requires
        unsafe { self.dispatch(gb, |checker, cx| checker.on_exec(cx, pc, opcode)) };
    }

    /// # Safety
    ///
    /// Same as [`Hooks::dispatch`].
    unsafe fn on_read(&mut self, gb: *mut GB_gameboy_t, addr: u16, value: u8) {
        let kind = if !self.fetched {
            // nothing has executed yet this step, so this is the opcode fetch; the execution
            // callback that follows it confirms the address
            self.pc = addr;
            ReadKind::Opcode
        } else if self.fetch_reads_remaining > 0 {
            self.fetch_reads_remaining -= 1;
            if self.operand_bytes_remaining > 0 {
                self.operand_bytes_remaining -= 1;
                self.trace.push_operand(value);
            }
            ReadKind::Operand
        } else {
            ReadKind::Data
        };

        // SAFETY: the caller's guarantees are the ones `dispatch` requires
        unsafe { self.dispatch(gb, |checker, cx| checker.on_read(cx, addr, value, kind)) };
    }

    /// # Safety
    ///
    /// Same as [`Hooks::dispatch`].
    unsafe fn on_write(&mut self, gb: *mut GB_gameboy_t, addr: u16, value: u8) {
        if matches!(addr, 0x8000..=0x9FFF | 0xFE00..=0xFEFF) {
            // SameBoy lets the PPU lag behind the CPU and only catches it up as part of a
            // video memory access, which for a write is after this callback. Catching it up
            // here, as the write is about to anyway, lets the checkers see the PPU state the
            // write will be judged against.
            // SAFETY: `GB_display_run` requires an initialized `GB_gameboy_t`, guaranteed by
            // the caller
            unsafe { GB_display_run(gb, 0, true) };
        }
        // SAFETY: the caller's guarantees are the ones `dispatch` requires
        unsafe { self.dispatch(gb, |checker, cx| checker.on_write(cx, addr, value)) };
    }
}

/// The encoded length in bytes of the instruction starting with `opcode`.
const fn instruction_len(opcode: u8) -> u8 {
    match opcode {
        // LD rr, d16
        0x01 | 0x11 | 0x21 | 0x31
        // LD [a16], SP
        | 0x08
        // JP a16 / JP cc, a16
        | 0xC2 | 0xC3 | 0xCA | 0xD2 | 0xDA
        // CALL a16 / CALL cc, a16
        | 0xC4 | 0xCC | 0xCD | 0xD4 | 0xDC
        // LD [a16], A / LD A, [a16]
        | 0xEA | 0xFA => 3,
        // LD r, d8
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E
        // STOP
        | 0x10
        // JR r8 / JR cc, r8
        | 0x18 | 0x20 | 0x28 | 0x30 | 0x38
        // ALU A, d8
        | 0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE
        // CB prefix
        | 0xCB
        // LDH [a8], A / LDH A, [a8]
        | 0xE0 | 0xF0
        // ADD SP, r8 / LD HL, SP+r8
        | 0xE8 | 0xF8 => 2,
        _ => 1,
    }
}

/// The most reads the CPU makes at `PC` after fetching `opcode`, before any read the
/// instruction performs on its operands.
const fn fetch_reads(opcode: u8) -> u8 {
    match opcode {
        // STOP looks at the byte after it once, then fetches it unless an interrupt is pending
        0x10 => 2,
        // HALT looks at the byte after it without consuming it
        0x76 => 1,
        _ => instruction_len(opcode) - 1,
    }
}

/// Install the callbacks on `gb`, with `hooks` as the state they record into.
///
/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`. `hooks` must stay valid, and must not be
/// accessed while `gb` is running, until the callbacks are removed or `gb` is freed.
pub(crate) unsafe fn install(gb: *mut GB_gameboy_t, hooks: *mut Hooks) {
    // SAFETY: the caller guarantees `gb` is initialized, which is all these setters require.
    // The callbacks are valid `extern "C" fn`s matching the callback signatures.
    unsafe {
        GB_set_user_data(gb, hooks.cast());
        GB_set_execution_callback(gb, Some(execution_callback));
        GB_set_read_memory_callback(gb, Some(read_memory_callback));
        GB_set_write_memory_callback(gb, Some(write_memory_callback));
    }
}

/// Run `event` against the [`Hooks`] behind `gb`'s user data pointer, holding on to a panic
/// rather than letting it unwind into SameBoy.
///
/// # Safety
///
/// Must only be called from a callback installed by [`install`].
unsafe fn with_hooks(gb: *mut GB_gameboy_t, event: impl FnOnce(&mut Hooks)) {
    // SAFETY: SameBoy passes callbacks the `GB_gameboy_t` they were installed on, and
    // `install` set its user data to a `Hooks` that nothing else accesses while `gb` runs
    let hooks = unsafe { &mut *GB_get_user_data(gb).cast::<Hooks>() };
    if hooks.panic.is_some() {
        return;
    }
    if let Err(panic) = catch_unwind(AssertUnwindSafe(|| event(hooks))) {
        hooks.panic = Some(panic);
    }
}

unsafe extern "C" fn execution_callback(gb: *mut GB_gameboy_t, address: u16, opcode: u8) {
    // SAFETY: this is a callback installed by `install`, and SameBoy does not hold `gb`
    // through a Rust reference
    unsafe { with_hooks(gb, |hooks| hooks.on_exec(gb, address, opcode)) };
}

unsafe extern "C" fn read_memory_callback(gb: *mut GB_gameboy_t, addr: u16, data: u8) -> u8 {
    // SAFETY: this is a callback installed by `install`, and SameBoy does not hold `gb`
    // through a Rust reference
    unsafe { with_hooks(gb, |hooks| hooks.on_read(gb, addr, data)) };
    data
}

unsafe extern "C" fn write_memory_callback(gb: *mut GB_gameboy_t, addr: u16, data: u8) -> bool {
    // SAFETY: this is a callback installed by `install`, and SameBoy does not hold `gb`
    // through a Rust reference
    unsafe { with_hooks(gb, |hooks| hooks.on_write(gb, addr, data)) };
    // never block the write
    true
}

/// Read a byte of memory without the read reaching the checkers.
///
/// `GB_safe_read_memory` avoids hardware side effects but still fires the read callback, so
/// the callback is lifted for the duration of the read.
///
/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t` with the callbacks from [`install`]
/// installed.
pub(crate) unsafe fn peek(gb: *mut GB_gameboy_t, addr: u16) -> u8 {
    // SAFETY: the caller guarantees `gb` is initialized, which is all these functions require
    unsafe {
        GB_set_read_memory_callback(gb, None);
        let value = GB_safe_read_memory(gb, addr);
        GB_set_read_memory_callback(gb, Some(read_memory_callback));
        value
    }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn registers(gb: *mut GB_gameboy_t) -> Registers {
    // SAFETY: `GB_get_registers` requires an initialized `GB_gameboy_t`, guaranteed by the
    // caller, and returns a pointer into it. Every bit pattern is a valid set of registers.
    let registers = unsafe { (*GB_get_registers(gb)).__bindgen_anon_1 };
    Registers {
        af: registers.af,
        bc: registers.bc,
        de: registers.de,
        hl: registers.hl,
        sp: registers.sp,
        pc: registers.pc,
    }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn ime(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_2.__bindgen_anon_1.ime != 0 }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn halted(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_2.__bindgen_anon_1.halted }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn halt_bug(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_2.__bindgen_anon_1.halt_bug }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn is_cgb(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: `GB_is_cgb` requires an initialized `GB_gameboy_t`, guaranteed by the caller
    unsafe { GB_is_cgb(gb) }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn oam_read_blocked(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_9.__bindgen_anon_1.oam_read_blocked }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn oam_write_blocked(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_9.__bindgen_anon_1.oam_write_blocked }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn vram_read_blocked(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_9.__bindgen_anon_1.vram_read_blocked }
}

/// # Safety
///
/// `gb` must point to an initialized `GB_gameboy_t`.
pub(crate) unsafe fn vram_write_blocked(gb: *const GB_gameboy_t) -> bool {
    // SAFETY: the caller guarantees `gb` is initialized; the section unions only overlay a
    // marker byte on their fields
    unsafe { (*gb).__bindgen_anon_9.__bindgen_anon_1.vram_write_blocked }
}
