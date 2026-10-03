//! Tests for the HALT hazard checker, driven by the scenarios in `roms/halt.asm`.

use quokka_lang_sameboy::{HaltHazard, Model, RunOutcome, Sameboy, Violation};
use std::sync::OnceLock;

const ROM: &[u8] = include_bytes!("roms/halt.gb");

// addresses from the listing in `roms/halt.asm`
const VBLANK: u16 = 0x0040;
const BUG: u16 = 0x0200;
const BUG_HALT: u16 = 0x0209;
const BUG_DONE: u16 = 0x020B;
const HANG: u16 = 0x0220;
const HANG_HALT: u16 = 0x0224;
const HANG_DONE: u16 = 0x0225;
const HANG_IME: u16 = 0x0230;
const HANG_IME_HALT: u16 = 0x0235;
const HANG_IME_DONE: u16 = 0x0236;
const HANG_UNUSED_BITS: u16 = 0x0240;
const HANG_UNUSED_BITS_HALT: u16 = 0x0249;
const HANG_UNUSED_BITS_DONE: u16 = 0x024A;
const DISPATCH: u16 = 0x0250;
const DISPATCH_HALT: u16 = 0x0257;
const WAIT: u16 = 0x0260;
const WAIT_DONE: u16 = 0x0269;
const WAIT_IME: u16 = 0x0270;
const WAIT_IME_HALT: u16 = 0x0279;
const MASKED: u16 = 0x0280;
const MASKED_DONE: u16 = 0x028A;

/// Enough instructions to sit through a whole frame halted, waiting for VBlank.
const FRAME: u32 = 100_000;

/// An emulator with the test ROM loaded and the checker installed, past the boot ROM and
/// about to execute `pc`.
fn emulator(model: Model, pc: u16) -> Sameboy {
    static DMG: OnceLock<Vec<u8>> = OnceLock::new();
    static CGB: OnceLock<Vec<u8>> = OnceLock::new();
    let cell = match model {
        Model::Dmg => &DMG,
        Model::Cgb => &CGB,
        Model::Mgb => unimplemented!("no test runs on MGB"),
    };
    let booted = cell.get_or_init(|| {
        let mut emulator = Sameboy::new(model);
        emulator.load_rom(ROM);
        assert_eq!(
            emulator.run_until_pc(0x0100, 10_000_000),
            RunOutcome::Reached,
            "boot ROM did not hand over to the cartridge"
        );
        emulator.save_state()
    });
    let mut emulator = Sameboy::new(model);
    emulator.load_rom(ROM);
    assert!(emulator.load_state(booted), "failed to restore boot state");
    emulator.add_checker(Box::new(HaltHazard::new()));
    emulator.write_pc(pc);
    emulator
}

fn bug(pc: u16) -> Violation {
    Violation {
        checker: "halt",
        kind: "bug",
        pc,
        address: Some(pc + 1),
        detail: "HALT with IME=0, IE=$01 and IF=$01: the next byte is executed twice".to_owned(),
    }
}

fn hang(pc: u16) -> Violation {
    Violation {
        checker: "halt",
        kind: "hang",
        pc,
        address: None,
        detail: "HALT with no interrupt enabled in IE: nothing can wake the CPU".to_owned(),
    }
}

#[test]
fn halt_with_interrupts_disabled_and_one_pending_is_reported() {
    for model in [Model::Dmg, Model::Cgb] {
        let mut emulator = emulator(model, BUG);

        // stops straight after the HALT, before the byte after it has run at all
        assert_eq!(
            emulator.run_until_pc(BUG_DONE, 100),
            RunOutcome::Violation,
            "{model:?}"
        );
        assert_eq!(emulator.violations(), [bug(BUG_HALT)], "{model:?}");
        assert_eq!(emulator.read_pc(), BUG_HALT + 1);
        assert!(emulator.halt_bug());
        assert!(!emulator.halted());

        // the bug itself goes ahead: `inc b` runs twice
        emulator.run();
        emulator.run();
        assert_eq!(emulator.read_pc(), BUG_DONE);
        assert_eq!(emulator.read_b(), 2);
        assert_eq!(emulator.finish(), [bug(BUG_HALT)], "{model:?}");
    }
}

#[test]
fn halt_with_no_interrupt_enabled_is_reported() {
    let cases = [
        (HANG, HANG_HALT, HANG_DONE),
        (HANG_IME, HANG_IME_HALT, HANG_IME_DONE),
        (
            HANG_UNUSED_BITS,
            HANG_UNUSED_BITS_HALT,
            HANG_UNUSED_BITS_DONE,
        ),
    ];
    for (start, halt, done) in cases {
        let mut emulator = emulator(Model::Dmg, start);

        // reported at the HALT rather than by running out of instructions
        assert_eq!(
            emulator.run_until_pc(done, FRAME),
            RunOutcome::Violation,
            "scenario at ${start:04X}"
        );
        assert_eq!(emulator.violations(), [hang(halt)]);
        assert!(emulator.halted());

        // and the CPU really is stuck
        emulator.run_for(FRAME);
        assert!(emulator.halted());
        assert_eq!(emulator.finish(), [hang(halt)]);
    }
}

#[test]
fn halt_with_interrupts_enabled_and_one_pending_is_clean() {
    let mut emulator = emulator(Model::Dmg, DISPATCH);

    assert_eq!(emulator.run_until_pc(VBLANK, 100), RunOutcome::Reached);
    assert_eq!(emulator.finish(), []);
    // the interrupt returns to the HALT, not past it
    let sp = emulator.read_sp();
    let return_address =
        u16::from_le_bytes([emulator.read_memory(sp), emulator.read_memory(sp + 1)]);
    assert_eq!(return_address, DISPATCH_HALT);
}

#[test]
fn halt_waiting_for_an_interrupt_with_interrupts_disabled_is_clean() {
    for (start, done) in [(WAIT, WAIT_DONE), (MASKED, MASKED_DONE)] {
        let mut emulator = emulator(Model::Dmg, start);

        // woken by VBlank without taking the interrupt
        assert_eq!(
            emulator.run_until_pc(done, FRAME),
            RunOutcome::Reached,
            "scenario at ${start:04X}"
        );
        assert_eq!(emulator.finish(), []);
        assert!(!emulator.ime());
    }
}

#[test]
fn halt_waiting_for_an_interrupt_with_interrupts_enabled_is_clean() {
    let mut emulator = emulator(Model::Dmg, WAIT_IME);

    assert_eq!(emulator.run_until_pc(VBLANK, FRAME), RunOutcome::Reached);
    assert_eq!(emulator.finish(), []);
    // the CPU did halt, so the interrupt returns past the HALT
    let sp = emulator.read_sp();
    let return_address =
        u16::from_le_bytes([emulator.read_memory(sp), emulator.read_memory(sp + 1)]);
    assert_eq!(return_address, WAIT_IME_HALT + 1);
}
