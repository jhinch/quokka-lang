//! Tests for the OAM corruption hazard checker, driven by the scenarios in `roms/oam.asm`.

use quokka_lang_sameboy::{Model, OamCorruptionHazard, RunOutcome, Sameboy, Violation};
use std::sync::OnceLock;

const ROM: &[u8] = include_bytes!("roms/oam.gb");

// addresses from the listing in `roms/oam.asm`
const VBLANK: u16 = 0x0040;
const INC_DEC: u16 = 0x0200;
const HL_STEP: u16 = 0x0210;
const LD_SP_HL: u16 = 0x0218;
const PUSH: u16 = 0x0220;
const CALL: u16 = 0x0221;
const CALL_Z: u16 = 0x0224;
const CALL_NC: u16 = 0x0227;
const RST: u16 = 0x022A;
const POP: u16 = 0x0230;
const RET: u16 = 0x0231;
const RETI: u16 = 0x0232;
const RET_Z: u16 = 0x0233;
const RET_NC: u16 = 0x0234;
const INTERRUPT: u16 = 0x0240;
const STRADDLE: u16 = 0x0270;
const JR_INTO_OAM: u16 = 0x0280;

const ZERO_FLAG: u8 = 0x80;
const CARRY_FLAG: u8 = 0x10;

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
    emulator.add_checker(Box::new(OamCorruptionHazard::new()));
    emulator
}

/// Runs the single instruction at `pc` once `setup` has arranged the registers, and returns
/// the violations it caused.
fn step(pc: u16, setup: impl FnOnce(&mut Sameboy)) -> Vec<Violation> {
    let mut emulator = emulator(Model::Dmg);
    setup(&mut emulator);
    emulator.write_pc(pc);
    emulator.run();
    emulator.finish().to_vec()
}

fn violation(kind: &'static str, pc: u16, address: u16, detail: &str) -> Violation {
    Violation {
        checker: "oam-corruption",
        kind,
        pc,
        address: Some(address),
        detail: detail.to_owned(),
    }
}

/// The `INC rr` / `DEC rr` scenarios: the offset of each from `INC_DEC`, how to set its
/// register, and how the violation names it.
#[allow(clippy::type_complexity)]
const INC_DEC_CASES: [(u16, fn(&mut Sameboy, u16), &str, &str); 8] = [
    (0, Sameboy::write_bc, "INC", "BC"),
    (1, Sameboy::write_de, "INC", "DE"),
    (2, Sameboy::write_hl, "INC", "HL"),
    (3, Sameboy::write_sp, "INC", "SP"),
    (4, Sameboy::write_bc, "DEC", "BC"),
    (5, Sameboy::write_de, "DEC", "DE"),
    (6, Sameboy::write_hl, "DEC", "HL"),
    (7, Sameboy::write_sp, "DEC", "SP"),
];

#[test]
fn inc_dec_of_a_register_in_oam_is_reported() {
    for (offset, write, mnemonic, name) in INC_DEC_CASES {
        for value in [0xFE00, 0xFE40, 0xFEFF] {
            let pc = INC_DEC + offset;
            assert_eq!(
                step(pc, |emulator| write(emulator, value)),
                [violation(
                    "inc-dec",
                    pc,
                    value,
                    &format!("{mnemonic} {name} with {name}=${value:04X}")
                )],
            );
        }
    }
}

#[test]
fn inc_dec_of_a_register_outside_oam_is_clean() {
    for (offset, write, mnemonic, name) in INC_DEC_CASES {
        // either side of OAM, including stepping into it: the bus sees the value going in
        for value in [0xFDFF, 0xFF00, 0xC000] {
            assert_eq!(
                step(INC_DEC + offset, |emulator| write(emulator, value)),
                [],
                "{mnemonic} {name} with {name}=${value:04X}"
            );
        }
    }
}

#[test]
fn hl_post_step_in_oam_is_reported() {
    let cases = [
        (0, "LD [HL+], A"),
        (1, "LD A, [HL+]"),
        (2, "LD [HL-], A"),
        (3, "LD A, [HL-]"),
    ];
    for (offset, mnemonic) in cases {
        let pc = HL_STEP + offset;
        assert_eq!(
            step(pc, |emulator| emulator.write_hl(0xFE40)),
            [violation(
                "hl-step",
                pc,
                0xFE40,
                &format!("{mnemonic} with HL=$FE40")
            )],
        );
        assert_eq!(step(pc, |emulator| emulator.write_hl(0xC000)), []);
    }
}

#[test]
fn ld_sp_hl_with_hl_in_oam_is_reported() {
    assert_eq!(
        step(LD_SP_HL, |emulator| emulator.write_hl(0xFE40)),
        [violation(
            "ld-sp-hl",
            LD_SP_HL,
            0xFE40,
            "LD SP, HL with HL=$FE40"
        )],
    );
    assert_eq!(step(LD_SP_HL, |emulator| emulator.write_hl(0xC000)), []);
}

#[test]
fn pushing_through_oam_is_reported() {
    let cases = [
        (PUSH, "PUSH", 0),
        (CALL, "CALL", 0),
        (CALL_Z, "CALL", ZERO_FLAG),
        (CALL_NC, "CALL", 0),
        (RST, "RST", 0),
    ];
    for (pc, mnemonic, flags) in cases {
        let setup = |sp: u16| {
            move |emulator: &mut Sameboy| {
                emulator.write_f(flags);
                emulator.write_sp(sp);
            }
        };
        assert_eq!(
            step(pc, setup(0xFE80)),
            [violation(
                "stack",
                pc,
                0xFE80,
                &format!("{mnemonic} with SP=$FE80")
            )],
        );
        // the second byte is written to SP - 2
        assert_eq!(
            step(pc, setup(0xFF01)),
            [violation(
                "stack",
                pc,
                0xFEFF,
                &format!("{mnemonic} with SP=$FF01")
            )],
        );
        assert_eq!(step(pc, setup(0xFF02)), [], "{mnemonic} with SP=$FF02");
        assert_eq!(step(pc, setup(0xFDFF)), [], "{mnemonic} with SP=$FDFF");
    }
}

#[test]
fn popping_through_oam_is_reported() {
    let cases = [
        (POP, "POP", 0),
        (RET, "RET", 0),
        (RETI, "RETI", 0),
        (RET_Z, "RET", ZERO_FLAG),
        (RET_NC, "RET", 0),
    ];
    for (pc, mnemonic, flags) in cases {
        let setup = |sp: u16| {
            move |emulator: &mut Sameboy| {
                emulator.write_f(flags);
                emulator.write_sp(sp);
            }
        };
        assert_eq!(
            step(pc, setup(0xFE80)),
            [violation(
                "stack",
                pc,
                0xFE80,
                &format!("{mnemonic} with SP=$FE80")
            )],
        );
        // the second byte is read from SP + 1
        assert_eq!(
            step(pc, setup(0xFDFF)),
            [violation(
                "stack",
                pc,
                0xFE00,
                &format!("{mnemonic} with SP=$FDFF")
            )],
        );
        assert_eq!(step(pc, setup(0xFDFE)), [], "{mnemonic} with SP=$FDFE");
        assert_eq!(step(pc, setup(0xFF00)), [], "{mnemonic} with SP=$FF00");
    }
}

#[test]
fn conditional_calls_and_returns_not_taken_are_clean() {
    let cases = [
        (CALL_Z, 0),
        (CALL_NC, CARRY_FLAG),
        (RET_Z, 0),
        (RET_NC, CARRY_FLAG),
    ];
    for (pc, flags) in cases {
        let violations = step(pc, |emulator| {
            emulator.write_f(flags);
            emulator.write_sp(0xFE80);
        });
        assert_eq!(violations, [], "instruction at ${pc:04X}");
    }
}

#[test]
fn interrupt_dispatch_with_sp_in_oam_is_reported() {
    let mut emulator = emulator(Model::Dmg);
    emulator.write_sp(0xFE80);
    emulator.write_pc(INTERRUPT);

    assert_eq!(emulator.run_until_pc(VBLANK, 100), RunOutcome::Violation);
    assert_eq!(
        emulator.violations(),
        [violation(
            "stack",
            // the instruction the interrupt was taken in front of
            INTERRUPT + 8,
            0xFE80,
            "interrupt dispatch with SP=$FE80"
        )],
    );
    // the dispatch itself went ahead
    assert_eq!(emulator.read_pc(), VBLANK);
    assert_eq!(emulator.read_sp(), 0xFE7E);
}

#[test]
fn interrupt_dispatch_with_sp_outside_oam_is_clean() {
    let mut emulator = emulator(Model::Dmg);
    emulator.write_sp(0xDFF0);
    emulator.write_pc(INTERRUPT);

    assert_eq!(emulator.run_until_pc(VBLANK, 100), RunOutcome::Reached);
    assert_eq!(emulator.finish(), []);
}

#[test]
fn executing_from_oam_is_reported() {
    // whatever OAM holds is executed, so only the fetch itself is checked
    let violations = step(0xFE10, |_| {});
    assert_eq!(
        violations.first(),
        Some(&violation("pc", 0xFE10, 0xFE10, "opcode fetched from OAM"))
    );
}

#[test]
fn instruction_running_into_oam_is_reported() {
    let mut emulator = emulator(Model::Dmg);
    emulator.write_pc(STRADDLE);

    assert_eq!(emulator.run_until_pc(0x7FFF, 100), RunOutcome::Violation);
    assert_eq!(
        emulator.violations(),
        [violation("pc", 0xFDFE, 0xFE00, "operand fetched from OAM")],
    );
}

#[test]
fn jr_stepping_pc_into_oam_is_reported() {
    let mut emulator = emulator(Model::Dmg);
    emulator.write_pc(JR_INTO_OAM);

    assert_eq!(emulator.run_until_pc(0x7FFF, 100), RunOutcome::Violation);
    assert_eq!(
        emulator.violations(),
        [violation(
            "pc",
            0xFDFE,
            0xFE00,
            "JR with PC stepping into OAM"
        )],
    );
}

#[test]
fn hazards_are_reported_on_cgb_too() {
    // CGB hardware does not have the bug, but the same code would corrupt OAM on a DMG
    let mut emulator = emulator(Model::Cgb);
    emulator.write_hl(0xFE40);
    emulator.write_pc(INC_DEC + 2);
    emulator.run();

    assert_eq!(
        emulator.finish(),
        [violation(
            "inc-dec",
            INC_DEC + 2,
            0xFE40,
            "INC HL with HL=$FE40"
        )],
    );
}
