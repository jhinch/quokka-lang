#![deny(clippy::unnecessary_semicolon)]
#![deny(clippy::needless_continue)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(clippy::must_use_candidate)]
#![deny(clippy::return_self_not_must_use)]
#![deny(clippy::cast_possible_truncation)]
#![deny(clippy::manual_assert)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod check;
mod hooks;
mod trace;

pub use check::{Checker, Context, ReadKind, Registers, Violation};
pub use trace::{Trace, TraceEntry};

use hooks::Hooks;
use quokka_lang_sameboy_sys::*;
use std::pin::Pin;
use std::ptr::NonNull;

pub struct Sameboy {
    inner: Pin<Box<GB_gameboy_t>>,
    pixels: (*mut u32, usize),
    /// The state SameBoy's callbacks record into, owned by this struct and freed on drop. It
    /// is held as a raw pointer because the callbacks reach it through SameBoy's user data
    /// pointer while the emulator runs.
    hooks: NonNull<Hooks>,
}

/// Why [`Sameboy::run_until_pc`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum RunOutcome {
    /// `PC` reached the target address.
    Reached,
    /// The instruction budget ran out first.
    InstructionLimit,
    /// A [`Checker`] reported a violation; see [`Sameboy::violations`].
    Violation,
}

impl Sameboy {
    #[must_use]
    pub fn new(model: Model) -> Self {
        let boot_rom = match model {
            Model::Dmg => bootroms::dmg(),
            Model::Mgb => bootroms::mgb(),
            Model::Cgb => bootroms::cgb(),
        };
        let model = model as GB_model_t;
        let mut inner = Box::<GB_gameboy_t>::new_uninit();
        // SAFETY: `GB_init` requires memory is `sizeof(GB_gameboy_t)` which
        // `Box::<GB_gameboy_t>::new_uninit()` guarantees
        unsafe {
            GB_init(inner.as_mut_ptr(), model);
        }
        // SAFETY: GB_init guarantees to initialize the memory
        let inner = unsafe { inner.assume_init() };
        let mut inner = Box::into_pin(inner);

        // SAFETY: `GB_set_log_callback` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees. `log_callback` is a valid `extern "C" fn` matching
        // `GB_log_callback_t`'s signature for the lifetime of `inner`.
        unsafe {
            GB_set_log_callback(inner.as_mut().get_mut(), Some(log_callback));
        }

        // SAFETY: `GB_get_screen_width` and `GB_get_screen_height` require `*mut GB_gameboy_t`
        // to point to initialized memory which Box<_> guarantees
        let (screen_width, screen_height): (u32, u32) = unsafe {
            (
                GB_get_screen_width(inner.as_mut().get_mut()),
                GB_get_screen_height(inner.as_mut().get_mut()),
            )
        };
        let size = (screen_height as usize)
            .checked_mul(screen_width as usize)
            .expect("Screen too large");
        let mut pixels = vec![0u32; size];
        // this shouldn't be needed, but guarantees capacity() == len()
        pixels.shrink_to_fit();
        // leaks the vec, but we will reconstruct it within Drop
        let (pixels_ptr, len, capacity) = pixels.into_raw_parts();
        debug_assert_eq!(len, capacity);
        // SAFETY: `GB_set_pixels_output` requires `*mut GB_gameboy_t` to point to initialized
        // memory and `*mut 32` to point to a buffer of size `height * width` which Box<_> and Vec<u8>
        // guarantee respectively. pixels_ptr has only just been leaked so it is valid.
        unsafe {
            GB_set_pixels_output(inner.as_mut().get_mut(), pixels_ptr);
        }

        let hooks = NonNull::from(Box::leak(Box::new(Hooks::new())));
        // SAFETY: `inner` is initialized, and `hooks` was only just leaked so it is valid. It
        // is freed in `Drop` after `inner`, and only accessed between runs.
        unsafe {
            hooks::install(inner.as_mut().get_mut(), hooks.as_ptr());
        }

        let mut sameboy = Sameboy {
            inner,
            pixels: (pixels_ptr, len),
            hooks,
        };

        sameboy.load_boot_rom(boot_rom);
        sameboy.reset();

        sameboy
    }

    pub fn reset(&mut self) {
        // SAFETY: `GB_reset` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_reset(self.inner.as_mut().get_mut());
        }
        self.hooks_mut().clear_history();
    }

    pub fn load_boot_rom(&mut self, rom: &[u8]) {
        // SAFETY: `GB_load_boot_rom_from_buffer` requires `*mut GB_gameboy_t` to point to
        // initialized memory and `*const u8` to point to a buffer of size `size` which
        // Box<_> and &[u8] guarantee respectively
        unsafe {
            GB_load_boot_rom_from_buffer(self.inner.as_mut().get_mut(), rom.as_ptr(), rom.len());
        }
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        // SAFETY: `GB_load_rom_from_buffer` requires `*mut GB_gameboy_t` to point to initialized
        // memory and `*const u8` to point to a buffer of size `size` which Box<_> and &[u8]
        // guarantee respectively
        unsafe {
            GB_load_rom_from_buffer(self.inner.as_mut().get_mut(), rom.as_ptr(), rom.len());
        }
    }

    /// Run one CPU instruction. Returns the time passed in 8MHz ticks.
    ///
    /// For example, if the next instruction is NOP, this function returns 8 if in normal
    /// mode and 4 in double speed (GBC) mode.
    ///
    /// # Panics
    ///
    /// Resumes any panic raised by a [`Checker`] while the instruction was running.
    pub fn run(&mut self) -> u32 {
        self.hooks_mut().begin_step();
        // SAFETY: `GB_run` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees. No reference to the hooks state is live, so the
        // callbacks have exclusive access to it.
        let ticks = unsafe { GB_run(self.inner.as_mut().get_mut()) };
        if let Some(panic) = self.hooks_mut().take_panic() {
            std::panic::resume_unwind(panic);
        }
        ticks
    }

    /// Run for at least the specified number of 8MHz ticks.
    pub fn run_for_cycles(&mut self, target: u32) {
        let mut value = 0;
        while value < target {
            value = value.checked_add(self.run()).expect("value overflowed");
        }
    }

    /// Run for a specified number of instructions
    pub fn run_for(&mut self, instr_count: u32) {
        for _ in 0..instr_count {
            self.run();
        }
    }

    /// Run until the `PC` register is equal to the `target` address. If `PC` is currently equal
    /// to `target`, execute the instructions until the next time `PC` is equal to the `target`.
    /// Stops early if a [`Checker`] reports a violation, after the instruction that caused it.
    pub fn run_until_pc(&mut self, target: u16, max_instr_count: u32) -> RunOutcome {
        for _ in 0..max_instr_count {
            self.run();
            if !self.violations().is_empty() {
                return RunOutcome::Violation;
            }
            if target == self.read_pc() {
                return RunOutcome::Reached;
            }
        }
        RunOutcome::InstructionLimit
    }

    /// Add a checker, which observes every instruction and memory access from now on.
    pub fn add_checker(&mut self, checker: Box<dyn Checker>) {
        self.hooks_mut().checkers.push(checker);
    }

    /// Run the end-of-run checks ([`Checker::on_finish`]) and return every violation recorded
    /// so far.
    pub fn finish(&mut self) -> &[Violation] {
        let gb: *mut GB_gameboy_t = self.inner.as_mut().get_mut();
        // SAFETY: `gb` points to initialized memory which Box<_> guarantees, and is not
        // accessed through `self.inner` while the checkers run
        unsafe {
            self.hooks_mut().finish(gb);
        }
        self.violations()
    }

    /// The violations the checkers have recorded so far, oldest first.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.hooks().violations
    }

    /// The most recently executed instructions.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.hooks().trace
    }

    /// Set how many instructions [`Sameboy::trace`] remembers. `0` turns tracing off.
    pub fn set_trace_capacity(&mut self, capacity: usize) {
        self.hooks_mut().trace.set_capacity(capacity);
    }

    fn hooks(&self) -> &Hooks {
        // SAFETY: `hooks` is valid until `Drop`. The callbacks only access it while `GB_run`
        // executes, which borrows `self` mutably.
        unsafe { self.hooks.as_ref() }
    }

    fn hooks_mut(&mut self) -> &mut Hooks {
        // SAFETY: `hooks` is valid until `Drop`. The callbacks only access it while `GB_run`
        // executes, by which point this borrow has ended.
        unsafe { self.hooks.as_mut() }
    }

    /// Capture a snapshot of the emulator's current state (CPU registers, RAM, MBC state, etc.)
    /// that can later be restored with [`Sameboy::load_state`].
    ///
    /// A snapshot does not include cartridge ROM contents, so it can be restored onto any
    /// `Sameboy` instance that has since had a compatible ROM (matching model, and an MBC RAM
    /// size no smaller than the snapshot's) loaded with [`Sameboy::load_rom`].
    #[must_use]
    pub fn save_state(&mut self) -> Vec<u8> {
        // SAFETY: `GB_get_save_state_size` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        let size = unsafe { GB_get_save_state_size(self.inner.as_mut().get_mut()) };
        let mut buffer = vec![0u8; size];
        // SAFETY: `GB_save_state_to_buffer` requires `*mut GB_gameboy_t` to point to initialized
        // memory, and `*mut u8` to point to a buffer of at least `GB_get_save_state_size(gb)`
        // bytes, both of which are guaranteed above
        unsafe {
            GB_save_state_to_buffer(self.inner.as_mut().get_mut(), buffer.as_mut_ptr());
        }
        buffer
    }

    /// Restore a snapshot previously captured with [`Sameboy::save_state`]. Returns `true` if
    /// the snapshot was compatible with the emulator's current state (model, currently loaded
    /// ROM's MBC RAM size, etc.) and was restored successfully.
    pub fn load_state(&mut self, state: &[u8]) -> bool {
        // SAFETY: `GB_load_state_from_buffer` requires `*mut GB_gameboy_t` to point to
        // initialized memory, and `*const u8`/`length` to describe a valid, readable buffer,
        // both of which are guaranteed by Box<_> and &[u8] respectively
        let result = unsafe {
            GB_load_state_from_buffer(self.inner.as_mut().get_mut(), state.as_ptr(), state.len())
        };
        self.hooks_mut().clear_history();
        result == 0
    }

    /// read a byte from emulator memory at the given address, without side effects: the read
    /// is not seen by any [`Checker`]
    pub fn read_memory(&mut self, addr: u16) -> u8 {
        // SAFETY: `hooks::peek` requires `*mut GB_gameboy_t` to point to initialized memory
        // with the callbacks installed, which Box<_> and `Sameboy::new` guarantee
        unsafe { hooks::peek(self.inner.as_mut().get_mut(), addr) }
    }

    /// read the current state of all the CPU registers
    pub fn registers(&mut self) -> Registers {
        // SAFETY: `hooks::registers` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe { hooks::registers(self.inner.as_mut().get_mut()) }
    }

    /// whether interrupts are enabled (the interrupt master enable flag)
    #[must_use]
    pub fn ime(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::ime(&raw const *self.inner) }
    }

    /// whether the CPU is halted, waiting for an interrupt
    #[must_use]
    pub fn halted(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::halted(&raw const *self.inner) }
    }

    /// whether the last `HALT` triggered the HALT bug, so the next opcode byte will be
    /// executed twice
    #[must_use]
    pub fn halt_bug(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::halt_bug(&raw const *self.inner) }
    }

    /// whether the PPU is currently blocking CPU reads of OAM
    #[must_use]
    pub fn oam_read_blocked(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::oam_read_blocked(&raw const *self.inner) }
    }

    /// whether the PPU is currently blocking CPU writes to OAM
    #[must_use]
    pub fn oam_write_blocked(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::oam_write_blocked(&raw const *self.inner) }
    }

    /// whether the PPU is currently blocking CPU reads of VRAM
    #[must_use]
    pub fn vram_read_blocked(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::vram_read_blocked(&raw const *self.inner) }
    }

    /// whether the PPU is currently blocking CPU writes to VRAM
    #[must_use]
    pub fn vram_write_blocked(&self) -> bool {
        // SAFETY: `inner` points to initialized memory which Box<_> guarantees
        unsafe { hooks::vram_write_blocked(&raw const *self.inner) }
    }

    /// read the current state of the `AF` register
    pub fn read_af(&mut self) -> u16 {
        self.read_r16(|registers| registers.af)
    }

    /// read the current state of the `BC` register
    pub fn read_bc(&mut self) -> u16 {
        self.read_r16(|registers| registers.bc)
    }

    /// read the current state of the `DE` register
    pub fn read_de(&mut self) -> u16 {
        self.read_r16(|registers| registers.de)
    }

    /// read the current state of the `HL` register
    pub fn read_hl(&mut self) -> u16 {
        self.read_r16(|registers| registers.hl)
    }

    /// read the current state of the `SP` register
    pub fn read_sp(&mut self) -> u16 {
        self.read_r16(|registers| registers.sp)
    }

    /// read the current state of the `PC` register
    pub fn read_pc(&mut self) -> u16 {
        self.read_r16(|registers| registers.pc)
    }

    /// read the current state of the `A` register
    pub fn read_a(&mut self) -> u8 {
        self.read_r8(|registers| registers.a)
    }

    /// read the current state of the `F` register
    pub fn read_f(&mut self) -> u8 {
        self.read_r8(|registers| registers.f)
    }

    /// read the current state of the `B` register
    pub fn read_b(&mut self) -> u8 {
        self.read_r8(|registers| registers.b)
    }

    /// read the current state of the `C` register
    pub fn read_c(&mut self) -> u8 {
        self.read_r8(|registers| registers.c)
    }

    /// read the current state of the `D` register
    pub fn read_d(&mut self) -> u8 {
        self.read_r8(|registers| registers.d)
    }

    /// read the current state of the `E` register
    pub fn read_e(&mut self) -> u8 {
        self.read_r8(|registers| registers.e)
    }

    /// read the current state of the `H` register
    pub fn read_h(&mut self) -> u8 {
        self.read_r8(|registers| registers.h)
    }

    /// read the current state of the `L` register
    pub fn read_l(&mut self) -> u8 {
        self.read_r8(|registers| registers.l)
    }

    /// write the `AF` register. The low four bits of `F` do not exist on hardware and are
    /// always written as zero.
    pub fn write_af(&mut self, value: u16) {
        self.write_r16(|registers| registers.af = value & 0xFFF0);
    }

    /// write the `BC` register
    pub fn write_bc(&mut self, value: u16) {
        self.write_r16(|registers| registers.bc = value);
    }

    /// write the `DE` register
    pub fn write_de(&mut self, value: u16) {
        self.write_r16(|registers| registers.de = value);
    }

    /// write the `HL` register
    pub fn write_hl(&mut self, value: u16) {
        self.write_r16(|registers| registers.hl = value);
    }

    /// write the `SP` register
    pub fn write_sp(&mut self, value: u16) {
        self.write_r16(|registers| registers.sp = value);
    }

    /// write the `PC` register, so the next instruction executed is the one at `value`
    pub fn write_pc(&mut self, value: u16) {
        self.write_r16(|registers| registers.pc = value);
    }

    /// write the `A` register
    pub fn write_a(&mut self, value: u8) {
        self.write_r8(|registers| registers.a = value);
    }

    /// write the `F` register. The low four bits do not exist on hardware and are always
    /// written as zero.
    pub fn write_f(&mut self, value: u8) {
        self.write_r8(|registers| registers.f = value & 0xF0);
    }

    /// write the `B` register
    pub fn write_b(&mut self, value: u8) {
        self.write_r8(|registers| registers.b = value);
    }

    /// write the `C` register
    pub fn write_c(&mut self, value: u8) {
        self.write_r8(|registers| registers.c = value);
    }

    /// write the `D` register
    pub fn write_d(&mut self, value: u8) {
        self.write_r8(|registers| registers.d = value);
    }

    /// write the `E` register
    pub fn write_e(&mut self, value: u8) {
        self.write_r8(|registers| registers.e = value);
    }

    /// write the `H` register
    pub fn write_h(&mut self, value: u8) {
        self.write_r8(|registers| registers.h = value);
    }

    /// write the `L` register
    pub fn write_l(&mut self, value: u8) {
        self.write_r8(|registers| registers.l = value);
    }

    fn write_r16(&mut self, f: impl FnOnce(&mut GB_registers_t__bindgen_ty_1)) {
        // SAFETY: `GB_get_registers` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        let registers_ptr = unsafe { GB_get_registers(self.inner.as_mut().get_mut()) };
        // SAFETY: `registers_ptr` is guaranteed to be initialized if `GB_gameboy_t` is
        // initialized, and every bit pattern is a valid set of registers
        unsafe { f(&mut (*registers_ptr).__bindgen_anon_1) }
    }

    fn write_r8(&mut self, f: impl FnOnce(&mut GB_registers_t__bindgen_ty_2)) {
        // SAFETY: `GB_get_registers` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        let registers_ptr = unsafe { GB_get_registers(self.inner.as_mut().get_mut()) };
        // SAFETY: `registers_ptr` is guaranteed to be initialized if `GB_gameboy_t` is
        // initialized, and every bit pattern is a valid set of registers
        unsafe { f(&mut (*registers_ptr).__bindgen_anon_2) }
    }

    fn read_r16<T>(&mut self, f: impl FnOnce(&GB_registers_t__bindgen_ty_1) -> T) -> T {
        // SAFETY: `GB_get_registers` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        let registers_ptr = unsafe { GB_get_registers(self.inner.as_mut().get_mut()) };
        // SAFETY: `registers_ptr` is guaranteed to be initialized if `GB_gameboy_t` is initialized
        unsafe { f(&(*registers_ptr).__bindgen_anon_1) }
    }

    fn read_r8<T>(&mut self, f: impl FnOnce(&GB_registers_t__bindgen_ty_2) -> T) -> T {
        // SAFETY: `GB_get_registers` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        let registers_ptr = unsafe { GB_get_registers(self.inner.as_mut().get_mut()) };
        // SAFETY: `registers_ptr` is guaranteed to be initialized if `GB_gameboy_t` is initialized
        unsafe { f(&(*registers_ptr).__bindgen_anon_2) }
    }
}

impl Drop for Sameboy {
    fn drop(&mut self) {
        // SAFETY: `GB_set_pixels_output` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_set_pixels_output(self.inner.as_mut().get_mut(), std::ptr::null_mut());
        }

        // SAFETY: We now have exclusive access to the pixel pointer as we nulled out the
        // reference passed to sameboy. We now can safely drop the memory
        drop(unsafe { Vec::from_raw_parts(self.pixels.0, self.pixels.1, self.pixels.1) });

        // SAFETY: `GB_free` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_free(self.inner.as_mut().get_mut());
        }

        // SAFETY: `hooks` was leaked from a Box in `Sameboy::new` and is freed nowhere else.
        // SameBoy has been freed, so its callbacks can no longer reach it.
        drop(unsafe { Box::from_raw(self.hooks.as_ptr()) });
    }
}

/// Receives log messages from SameBoy (registered via `GB_set_log_callback`) and forwards them
/// to `tracing` instead of letting SameBoy print them to stderr/stdout directly.
extern "C" fn log_callback(
    _gb: *mut GB_gameboy_t,
    string: *const std::os::raw::c_char,
    _attributes: GB_log_attributes_t,
) {
    // SAFETY: SameBoy guarantees `string` is a valid, non-null, NUL-terminated C string for
    // the duration of this call
    let message = unsafe { std::ffi::CStr::from_ptr(string) }.to_string_lossy();
    tracing::info!("{}", message.trim_end_matches('\n'));
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum Model {
    Dmg = GB_model_t_GB_MODEL_DMG_B,
    Mgb = GB_model_t_GB_MODEL_MGB,
    Cgb = GB_model_t_GB_MODEL_CGB_E,
}
