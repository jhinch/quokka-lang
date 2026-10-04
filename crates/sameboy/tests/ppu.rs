//! Tests for the PPU-blocked access checker, driven by the scenarios in `roms/ppu.asm`.

use quokka_lang_sameboy::{Model, PpuBlockedAccess, RunOutcome, Sameboy, Violation};
use std::sync::OnceLock;

const ROM: &[u8] = include_bytes!("roms/ppu.gb");

/// One scenario from the listing in `roms/ppu.asm`: where it starts, where its access to
/// `[HL]` is, and the loop it ends in.
#[derive(Debug, Clone, Copy)]
struct Scenario {
    start: u16,
    access: u16,
    done: u16,
}

const WRITE_DRAWING: Scenario = scenario(0x0200, 0x0210, 0x0212);
const READ_DRAWING: Scenario = scenario(0x0220, 0x0230, 0x0231);
const WRITE_OAM_SCAN: Scenario = scenario(0x0240, 0x0250, 0x0252);
const READ_OAM_SCAN: Scenario = scenario(0x0260, 0x0270, 0x0271);
const WRITE_HBLANK: Scenario = scenario(0x0280, 0x0290, 0x0292);
const READ_HBLANK: Scenario = scenario(0x02A0, 0x02B0, 0x02B1);
const WRITE_VBLANK: Scenario = scenario(0x02C0, 0x02C8, 0x02CA);
const READ_VBLANK: Scenario = scenario(0x02D0, 0x02D8, 0x02D9);
const WRITE_LCD_OFF: Scenario = scenario(0x02E0, 0x02F0, 0x02F2);
const READ_LCD_OFF: Scenario = scenario(0x0300, 0x0310, 0x0311);

const fn scenario(start: u16, access: u16, done: u16) -> Scenario {
    Scenario {
        start,
        access,
        done,
    }
}

const VRAM: u16 = 0x8000;
const OAM: u16 = 0xFE00;
const LCDC: u16 = 0xFF40;
const LY: u16 = 0xFF44;

/// The value the write scenarios store.
const WRITTEN: u8 = 0x42;
/// What `B` holds before a read scenario, so that the read is seen to replace it.
const UNREAD: u8 = 0x55;

/// Enough instructions to poll through a whole frame waiting for a PPU mode.
const FRAME: u32 = 100_000;

/// An emulator with the test ROM loaded and the checker installed, past the boot ROM.
fn emulator(model: Model) -> Sameboy {
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
    emulator.add_checker(Box::new(PpuBlockedAccess::new()));
    emulator
}

/// Reads `addr` at a point where the PPU is not blocking it: in VBlank, or with the LCD off.
fn read_unblocked(emulator: &mut Sameboy, addr: u16) -> u8 {
    for _ in 0..FRAME {
        if emulator.read_memory(LCDC) & 0x80 == 0 || emulator.read_memory(LY) == 145 {
            return emulator.read_memory(addr);
        }
        emulator.run();
    }
    panic!("the PPU never reached VBlank");
}

/// An address in the 16 bytes from `base` holding none of the `unwanted` values. OAM starts
/// out with random contents, so no single address is sure to hold a usable value.
fn address_not_holding(model: Model, base: u16, unwanted: &[u8]) -> u16 {
    let mut emulator = emulator(model);
    (base..base + 16)
        .find(|&addr| !unwanted.contains(&read_unblocked(&mut emulator, addr)))
        .expect("no address holds a usable value")
}

/// An address near `base` that a write of [`WRITTEN`] is seen to change.
fn writable(model: Model, base: u16) -> u16 {
    address_not_holding(model, base, &[WRITTEN])
}

/// An address near `base` that a read is seen to return the contents of, rather than `$FF` or
/// leaving `B` as it was.
fn readable(model: Model, base: u16) -> u16 {
    address_not_holding(model, base, &[0xFF, UNREAD])
}

/// Runs a write scenario against `addr` to its end. Returns how the run to the end of the
/// scenario went, and the emulator.
fn write(model: Model, scenario: Scenario, addr: u16) -> (RunOutcome, Sameboy) {
    let mut emulator = emulator(model);
    emulator.write_hl(addr);
    emulator.write_pc(scenario.start);
    let outcome = emulator.run_until_pc(scenario.done, FRAME);
    (outcome, emulator)
}

/// Runs a read scenario against `addr` to its end. Returns how the run to the end of the
/// scenario went, what the memory really held, and the emulator.
fn read(model: Model, scenario: Scenario, addr: u16) -> (RunOutcome, u8, Sameboy) {
    let mut emulator = emulator(model);
    let held = read_unblocked(&mut emulator, addr);
    emulator.write_hl(addr);
    emulator.write_b(UNREAD);
    emulator.write_pc(scenario.start);
    let outcome = emulator.run_until_pc(scenario.done, FRAME);
    (outcome, held, emulator)
}

fn violation(kind: &'static str, pc: u16, address: u16, detail: &str) -> Violation {
    Violation {
        checker: "ppu-blocked",
        kind,
        pc,
        address: Some(address),
        detail: detail.to_owned(),
    }
}

#[test]
fn vram_write_while_drawing_is_reported() {
    for model in [Model::Dmg, Model::Cgb] {
        let addr = writable(model, VRAM);
        let (outcome, mut emulator) = write(model, WRITE_DRAWING, addr);

        assert_eq!(outcome, RunOutcome::Violation, "{model:?}");
        let expected = [violation(
            "vram-write",
            WRITE_DRAWING.access,
            addr,
            "write of $42 to VRAM while the PPU is using it (mode 3): dropped",
        )];
        assert_eq!(emulator.violations(), expected, "{model:?}");
        // the write really was dropped
        assert_ne!(read_unblocked(&mut emulator, addr), WRITTEN, "{model:?}");
        assert_eq!(emulator.finish(), expected, "{model:?}");
    }
}

#[test]
fn vram_read_while_drawing_is_reported() {
    for model in [Model::Dmg, Model::Cgb] {
        let addr = readable(model, VRAM);
        let (outcome, _, mut emulator) = read(model, READ_DRAWING, addr);

        assert_eq!(outcome, RunOutcome::Violation, "{model:?}");
        assert_eq!(
            emulator.finish(),
            [violation(
                "vram-read",
                READ_DRAWING.access,
                addr,
                "read of VRAM while the PPU is using it (mode 3): returns $FF",
            )],
            "{model:?}"
        );
        assert_eq!(emulator.read_b(), 0xFF, "{model:?}");
    }
}

#[test]
fn oam_write_while_the_ppu_uses_it_is_reported() {
    for (scenario, mode) in [(WRITE_OAM_SCAN, 2), (WRITE_DRAWING, 3)] {
        for model in [Model::Dmg, Model::Cgb] {
            let addr = writable(model, OAM);
            let (outcome, mut emulator) = write(model, scenario, addr);

            assert_eq!(outcome, RunOutcome::Violation, "{model:?} mode {mode}");
            let expected = [violation(
                "oam-write",
                scenario.access,
                addr,
                &format!("write of $42 to OAM while the PPU is using it (mode {mode}): dropped"),
            )];
            assert_eq!(emulator.violations(), expected, "{model:?} mode {mode}");
            assert_ne!(read_unblocked(&mut emulator, addr), WRITTEN);
            assert_eq!(emulator.finish(), expected, "{model:?} mode {mode}");
        }
    }
}

#[test]
fn oam_read_while_the_ppu_uses_it_is_reported() {
    for (scenario, mode) in [(READ_OAM_SCAN, 2), (READ_DRAWING, 3)] {
        for model in [Model::Dmg, Model::Cgb] {
            let addr = readable(model, OAM);
            let (outcome, _, mut emulator) = read(model, scenario, addr);

            assert_eq!(outcome, RunOutcome::Violation, "{model:?} mode {mode}");
            assert_eq!(
                emulator.finish(),
                [violation(
                    "oam-read",
                    scenario.access,
                    addr,
                    &format!("read of OAM while the PPU is using it (mode {mode}): returns $FF"),
                )],
                "{model:?} mode {mode}"
            );
            assert_eq!(emulator.read_b(), 0xFF, "{model:?} mode {mode}");
        }
    }
}

#[test]
fn the_last_byte_of_each_region_is_covered() {
    let (outcome, mut emulator) = write(Model::Dmg, WRITE_DRAWING, 0x9FFF);
    assert_eq!(outcome, RunOutcome::Violation);
    assert_eq!(emulator.finish().len(), 1);

    let (outcome, mut emulator) = write(Model::Dmg, WRITE_DRAWING, 0xFE9F);
    assert_eq!(outcome, RunOutcome::Violation);
    assert_eq!(emulator.finish().len(), 1);
}

#[test]
fn memory_the_ppu_does_not_own_is_clean_while_drawing() {
    // work RAM, and the unusable area after the sprite table
    for addr in [0xC000, 0xFEA0] {
        let (outcome, mut emulator) = write(Model::Dmg, WRITE_DRAWING, addr);
        assert_eq!(outcome, RunOutcome::Reached, "${addr:04X}");
        assert_eq!(emulator.finish(), [], "${addr:04X}");
    }
}

#[test]
fn vram_access_during_the_oam_scan_is_clean() {
    let addr = writable(Model::Dmg, VRAM);
    let (outcome, mut emulator) = write(Model::Dmg, WRITE_OAM_SCAN, addr);
    assert_eq!(outcome, RunOutcome::Reached);
    assert_eq!(emulator.finish(), []);
    assert_eq!(read_unblocked(&mut emulator, addr), WRITTEN);

    let addr = readable(Model::Dmg, VRAM);
    let (outcome, held, mut emulator) = read(Model::Dmg, READ_OAM_SCAN, addr);
    assert_eq!(outcome, RunOutcome::Reached);
    assert_eq!(emulator.finish(), []);
    assert_eq!(emulator.read_b(), held);
}

#[test]
fn access_while_the_ppu_is_idle_is_clean() {
    let writes = [WRITE_HBLANK, WRITE_VBLANK, WRITE_LCD_OFF];
    let reads = [READ_HBLANK, READ_VBLANK, READ_LCD_OFF];
    for model in [Model::Dmg, Model::Cgb] {
        for base in [VRAM, OAM] {
            let addr = writable(model, base);
            for scenario in writes {
                let (outcome, mut emulator) = write(model, scenario, addr);
                let context = format!("{model:?} ${addr:04X} {scenario:?}");
                assert_eq!(outcome, RunOutcome::Reached, "{context}");
                assert_eq!(emulator.finish(), [], "{context}");
                assert_eq!(read_unblocked(&mut emulator, addr), WRITTEN, "{context}");
            }
            let addr = readable(model, base);
            for scenario in reads {
                let (outcome, held, mut emulator) = read(model, scenario, addr);
                let context = format!("{model:?} ${addr:04X} {scenario:?}");
                assert_eq!(outcome, RunOutcome::Reached, "{context}");
                assert_eq!(emulator.finish(), [], "{context}");
                assert_eq!(emulator.read_b(), held, "{context}");
            }
        }
    }
}
