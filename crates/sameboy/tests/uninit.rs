//! Tests for the uninitialized read checker, driven by the scenarios in `roms/uninit.asm`.

use quokka_lang_sameboy::{MemoryMap, Model, RunOutcome, Sameboy, UninitializedRead, Violation};
use std::sync::OnceLock;

const ROM: &[u8] = include_bytes!("roms/uninit.gb");

// addresses from the listing in `roms/uninit.asm`
const DONE: u16 = 0x0101;
const READ: u16 = 0x0200;
const WRITE_THEN_READ: u16 = 0x0210;
const ECHO: u16 = 0x0220;
const HIGH_RAM: u16 = 0x0230;
const OAM: u16 = 0x0240;
const ALWAYS_DEFINED: u16 = 0x0250;
const POP: u16 = 0x0260;
const RET: u16 = 0x0270;
const ADD_SP: u16 = 0x0280;
const INC_SP: u16 = 0x0290;
const SWITCH_STACK: u16 = 0x02A0;
const NARROW_STORE: u16 = 0x02C0;
const NARROW_READ_TWICE: u16 = 0x02D0;
const NARROW_READ_ONCE: u16 = 0x02E0;
const DMA: u16 = 0x0300;

/// The byte the narrow scenarios access.
const NARROW: u16 = 0xC010;

/// An emulator with the test ROM loaded and the checker installed, past the boot ROM.
fn emulator() -> Sameboy {
    static BOOTED: OnceLock<Vec<u8>> = OnceLock::new();
    let booted = BOOTED.get_or_init(|| {
        let mut emulator = Sameboy::new(Model::Dmg);
        emulator.load_rom(ROM);
        assert_eq!(
            emulator.run_until_pc(0x0100, 10_000_000),
            RunOutcome::Reached,
            "boot ROM did not hand over to the cartridge"
        );
        emulator.save_state()
    });
    let mut emulator = Sameboy::new(Model::Dmg);
    emulator.load_rom(ROM);
    assert!(emulator.load_state(booted), "failed to restore boot state");
    emulator.add_checker(Box::new(UninitializedRead::new()));
    emulator
}

/// Runs the scenario at `start` to its end, carrying on past every violation, and returns
/// them all along with the end-of-run ones.
fn run(mut emulator: Sameboy, start: u16) -> Vec<Violation> {
    emulator.write_pc(start);
    for _ in 0..1_000 {
        if emulator.read_pc() == DONE {
            return emulator.finish().to_vec();
        }
        emulator.run();
    }
    panic!("scenario at ${start:04X} did not finish");
}

fn scenario(start: u16) -> Vec<Violation> {
    run(emulator(), start)
}

/// Runs the scenario at `start` with the byte it accesses declared as narrow.
fn narrow_scenario(start: u16) -> Vec<Violation> {
    let mut memory_map = MemoryMap::new();
    memory_map.add_narrow(NARROW);
    let mut emulator = emulator();
    emulator.set_memory_map(memory_map);
    run(emulator, start)
}

fn violation(kind: &'static str, pc: u16, address: u16, detail: &str) -> Violation {
    Violation {
        checker: "uninitialized-read",
        kind,
        pc,
        address: Some(address),
        detail: detail.to_owned(),
    }
}

fn never_written(pc: u16, address: u16) -> Violation {
    violation(
        "never-written",
        pc,
        address,
        "read of memory which has not been written",
    )
}

fn released(pc: u16, address: u16) -> Violation {
    violation(
        "released",
        pc,
        address,
        "read of stack memory released when SP moved above it",
    )
}

#[test]
fn read_of_never_written_work_ram_is_reported() {
    assert_eq!(scenario(READ), [never_written(READ, 0xC000)]);
}

#[test]
fn read_of_written_work_ram_is_clean() {
    assert_eq!(scenario(WRITE_THEN_READ), []);
}

#[test]
fn run_stops_at_the_violation() {
    let mut emulator = emulator();
    emulator.write_pc(READ);
    assert_eq!(emulator.run_until_pc(DONE, 100), RunOutcome::Violation);
    assert_eq!(emulator.violations(), [never_written(READ, 0xC000)]);
}

#[test]
fn echo_shares_the_state_of_work_ram() {
    // $C000 was written through $E000, and $C001 is read through $E001
    assert_eq!(scenario(ECHO), [never_written(ECHO + 8, 0xE001)]);
}

#[test]
fn high_ram_is_tracked() {
    assert_eq!(scenario(HIGH_RAM), [never_written(HIGH_RAM, 0xFF80)]);
}

#[test]
fn oam_is_tracked() {
    assert_eq!(scenario(OAM), [never_written(OAM + 6, 0xFE00)]);
}

#[test]
fn rom_video_ram_cartridge_ram_and_io_registers_are_always_defined() {
    assert_eq!(scenario(ALWAYS_DEFINED), []);
}

#[test]
fn read_of_a_popped_stack_slot_is_reported() {
    // the read at +4 is of the slot while it is still pushed
    assert_eq!(scenario(POP), [released(POP + 8, 0xCFFF)]);
}

#[test]
fn read_of_a_return_address_after_the_return_is_reported() {
    assert_eq!(scenario(RET), [released(RET + 6, 0xCFFE)]);
}

#[test]
fn read_of_a_freed_frame_is_reported() {
    // the read at +8 is of the frame while it is still allocated
    assert_eq!(scenario(ADD_SP), [released(ADD_SP + 11, 0xCFFE)]);
}

#[test]
fn stepping_sp_releases_only_the_byte_it_passes() {
    // $CFFF is still on the stack
    assert_eq!(scenario(INC_SP), [released(INC_SP + 8, 0xCFFE)]);
}

#[test]
fn loading_sp_releases_nothing() {
    assert_eq!(scenario(SWITCH_STACK), []);
}

#[test]
fn a_byte_is_reported_once() {
    let mut emulator = emulator();
    for _ in 0..2 {
        emulator.write_pc(READ);
        emulator.run();
    }
    assert_eq!(emulator.finish(), [never_written(READ, 0xC000)]);
}

#[test]
fn narrow_store_is_clean() {
    assert_eq!(narrow_scenario(NARROW_STORE), []);
}

#[test]
fn narrow_byte_read_twice_before_a_write_is_reported() {
    assert_eq!(
        narrow_scenario(NARROW_READ_TWICE),
        [violation(
            "narrow",
            NARROW_READ_TWICE + 3,
            NARROW,
            "second read of a narrow byte which has not been written, first read at PC=$02D0"
        )],
    );
}

#[test]
fn narrow_byte_read_and_never_written_is_reported_at_the_end() {
    assert_eq!(
        narrow_scenario(NARROW_READ_ONCE),
        [violation(
            "narrow",
            DONE,
            NARROW,
            "narrow byte read at PC=$02E0 was never written"
        )],
    );
}

#[test]
fn bytes_are_only_narrow_if_the_memory_map_says_so() {
    assert_eq!(
        scenario(NARROW_STORE),
        [never_written(NARROW_STORE, NARROW)]
    );
}

#[test]
fn dma_transfer_defines_oam_from_its_source() {
    // of the page transferred only $C101 was written, so only $FE01 is defined
    assert_eq!(scenario(DMA), [never_written(DMA + 0x1B, 0xFE00)]);
}
