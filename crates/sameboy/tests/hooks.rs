//! Tests for the execution and memory hooks, driven by the scenarios in `roms/hooks.asm`.

use quokka_lang_sameboy::{
    Checker, Context, Model, ReadKind, Registers, RunOutcome, Sameboy, Violation,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

const ROM: &[u8] = include_bytes!("roms/hooks.gb");

// addresses from the listing in `roms/hooks.asm`
const EVENTS: u16 = 0x0150;
const EVENTS_DONE: u16 = 0x015A;
const FLAGS: u16 = 0x0200;
const HALT_BUG: u16 = 0x0210;
const HALT_BUG_INC_B: u16 = 0x021A;
const HALT_BUG_DONE: u16 = 0x021B;

/// An emulator with the test ROM loaded, past the boot ROM and about to execute `$0100`.
fn emulator() -> Sameboy {
    static BOOTED: OnceLock<Vec<u8>> = OnceLock::new();
    let booted = BOOTED.get_or_init(|| {
        let mut emulator = Sameboy::new(Model::Dmg);
        emulator.load_rom(ROM);
        assert_eq!(
            emulator.run_until_pc(0x0100, 1_000_000),
            RunOutcome::Reached,
            "boot ROM did not hand over to the cartridge"
        );
        emulator.save_state()
    });
    let mut emulator = Sameboy::new(Model::Dmg);
    emulator.load_rom(ROM);
    assert!(emulator.load_state(booted), "failed to restore boot state");
    emulator
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Exec(u16, u8),
    Read(u16, u8, ReadKind),
    Write(u16, u8),
}

/// Records every event it sees, along with the registers at each `on_exec`.
#[derive(Default)]
struct Recorder {
    events: Rc<RefCell<Vec<Event>>>,
    registers: Rc<RefCell<Vec<Registers>>>,
}

impl Checker for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }

    fn on_exec(&mut self, cx: &mut Context<'_>, pc: u16, opcode: u8) {
        assert_eq!(cx.pc(), pc);
        // peeking from inside a callback must not come back around as a read event
        let _ = cx.peek(0xC000);
        self.events.borrow_mut().push(Event::Exec(pc, opcode));
        self.registers.borrow_mut().push(cx.registers());
    }

    fn on_read(&mut self, _cx: &mut Context<'_>, addr: u16, value: u8, kind: ReadKind) {
        self.events
            .borrow_mut()
            .push(Event::Read(addr, value, kind));
    }

    fn on_write(&mut self, _cx: &mut Context<'_>, addr: u16, value: u8) {
        self.events.borrow_mut().push(Event::Write(addr, value));
    }
}

/// Reports a violation for every write to `address`.
struct NoWritesTo {
    address: u16,
}

impl Checker for NoWritesTo {
    fn name(&self) -> &'static str {
        "no-writes-to"
    }

    fn on_write(&mut self, cx: &mut Context<'_>, addr: u16, value: u8) {
        if addr == self.address {
            cx.report("forbidden-write", Some(addr), format!("wrote ${value:02X}"));
        }
    }
}

#[test]
fn events_are_observed_in_bus_order() {
    use Event::{Exec, Read, Write};
    use ReadKind::{Data, Opcode, Operand};

    let recorder = Recorder::default();
    let events = Rc::clone(&recorder.events);
    let mut emulator = emulator();
    emulator.add_checker(Box::new(recorder));

    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    assert_eq!(
        *events.borrow(),
        [
            // nop
            Read(0x0100, 0x00, Opcode),
            Exec(0x0100, 0x00),
            // jp $0150
            Read(0x0101, 0xC3, Opcode),
            Exec(0x0101, 0xC3),
            Read(0x0102, 0x50, Operand),
            Read(0x0103, 0x01, Operand),
            // ld a, $42
            Read(0x0150, 0x3E, Opcode),
            Exec(0x0150, 0x3E),
            Read(0x0151, 0x42, Operand),
            // ld [$C000], a
            Read(0x0152, 0xEA, Opcode),
            Exec(0x0152, 0xEA),
            Read(0x0153, 0x00, Operand),
            Read(0x0154, 0xC0, Operand),
            Write(0xC000, 0x42),
            // ld hl, $C000
            Read(0x0155, 0x21, Opcode),
            Exec(0x0155, 0x21),
            Read(0x0156, 0x00, Operand),
            Read(0x0157, 0xC0, Operand),
            // ld b, [hl]
            Read(0x0158, 0x46, Opcode),
            Exec(0x0158, 0x46),
            Read(0xC000, 0x42, Data),
            // inc [hl]
            Read(0x0159, 0x34, Opcode),
            Exec(0x0159, 0x34),
            Read(0xC000, 0x42, Data),
            Write(0xC000, 0x43),
        ]
    );
}

#[test]
fn on_exec_sees_registers_before_the_instruction_executes() {
    let recorder = Recorder::default();
    let registers = Rc::clone(&recorder.registers);
    let mut emulator = emulator();
    emulator.add_checker(Box::new(recorder));

    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    let registers = registers.borrow();
    // `ld a, $42` at $0150 has not loaded A yet; `ld [$C000], a` at $0152 sees it loaded
    let before_load = registers.iter().find(|r| r.pc == 0x0150).unwrap();
    let after_load = registers.iter().find(|r| r.pc == 0x0152).unwrap();
    assert_ne!(before_load.af >> 8, 0x42);
    assert_eq!(after_load.af >> 8, 0x42);
    // `ld hl, $C000` at $0155 has not loaded HL yet; `ld b, [hl]` at $0158 sees it loaded
    let before_load = registers.iter().find(|r| r.pc == 0x0155).unwrap();
    let after_load = registers.iter().find(|r| r.pc == 0x0158).unwrap();
    assert_ne!(before_load.hl, 0xC000);
    assert_eq!(after_load.hl, 0xC000);
}

#[test]
fn reading_memory_from_outside_is_not_an_event() {
    let recorder = Recorder::default();
    let events = Rc::clone(&recorder.events);
    let mut emulator = emulator();
    emulator.add_checker(Box::new(recorder));
    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    let before = events.borrow().len();
    assert_eq!(emulator.read_memory(0xC000), 0x43);
    assert_eq!(events.borrow().len(), before);

    // and the hooks still work afterwards
    emulator.run();
    assert_eq!(
        events.borrow()[before..],
        [
            Event::Read(0x015A, 0x18, ReadKind::Opcode),
            Event::Exec(0x015A, 0x18),
            Event::Read(0x015B, 0xFE, ReadKind::Operand),
        ]
    );
}

#[test]
fn violation_stops_the_run_and_is_recorded() {
    let mut emulator = emulator();
    emulator.add_checker(Box::new(NoWritesTo { address: 0xC000 }));

    assert_eq!(
        emulator.run_until_pc(EVENTS_DONE, 100),
        RunOutcome::Violation
    );

    assert_eq!(
        emulator.violations(),
        [Violation {
            checker: "no-writes-to",
            kind: "forbidden-write",
            pc: 0x0152,
            address: Some(0xC000),
            detail: "wrote $42".to_owned(),
        }]
    );
    assert_eq!(
        emulator.violations()[0].to_string(),
        "no-writes-to/forbidden-write at PC=$0152, address $C000: wrote $42"
    );
    // the run stopped right after the offending instruction, which is the last one traced
    assert_eq!(emulator.read_pc(), 0x0155);
    let last = emulator.trace().entries().last().unwrap();
    assert_eq!(last.pc, 0x0152);
    assert_eq!(last.bytes(), [0xEA, 0x00, 0xC0]);
}

#[test]
fn run_without_violations_reports_none() {
    let mut emulator = emulator();
    emulator.add_checker(Box::new(NoWritesTo { address: 0xD000 }));

    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);
    assert!(emulator.finish().is_empty());
}

#[test]
fn instruction_limit_is_reported() {
    let mut emulator = emulator();
    assert_eq!(
        emulator.run_until_pc(0x7FFF, 100),
        RunOutcome::InstructionLimit
    );
}

#[test]
fn on_finish_checks_run_against_the_final_state() {
    struct ExpectB(u8);

    impl Checker for ExpectB {
        fn name(&self) -> &'static str {
            "expect-b"
        }

        fn on_finish(&mut self, cx: &mut Context<'_>) {
            let b = cx.registers().bc >> 8;
            if b != u16::from(self.0) {
                cx.report("wrong-b", None, format!("B is ${b:02X}"));
            }
        }
    }

    let mut emulator = emulator();
    emulator.add_checker(Box::new(ExpectB(0x99)));
    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);
    assert!(emulator.violations().is_empty());

    assert_eq!(
        emulator.finish(),
        [Violation {
            checker: "expect-b",
            kind: "wrong-b",
            pc: EVENTS_DONE,
            address: None,
            detail: "B is $42".to_owned(),
        }]
    );
}

#[test]
#[should_panic(expected = "checker exploded")]
fn checker_panic_surfaces_from_run() {
    struct Explode;

    impl Checker for Explode {
        fn name(&self) -> &'static str {
            "explode"
        }

        fn on_write(&mut self, _cx: &mut Context<'_>, _addr: u16, _value: u8) {
            panic!("checker exploded");
        }
    }

    let mut emulator = emulator();
    emulator.add_checker(Box::new(Explode));
    let _ = emulator.run_until_pc(EVENTS_DONE, 100);
}

#[test]
fn trace_records_instruction_bytes_and_registers() {
    let mut emulator = emulator();
    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    let trace = emulator.trace();
    let bytes: Vec<(u16, &[u8])> = trace.entries().map(|e| (e.pc, e.bytes())).collect();
    assert_eq!(
        bytes,
        [
            (0x0100, &[0x00][..]),
            (0x0101, &[0xC3, 0x50, 0x01]),
            (0x0150, &[0x3E, 0x42]),
            (0x0152, &[0xEA, 0x00, 0xC0]),
            (0x0155, &[0x21, 0x00, 0xC0]),
            (0x0158, &[0x46]),
            (0x0159, &[0x34]),
        ]
    );

    let store = trace.entries().nth(3).unwrap();
    assert_eq!(store.registers.af >> 8, 0x42);
    let registers = store.registers;
    assert_eq!(
        store.to_string(),
        format!(
            "$0152  EA 00 C0  AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X}",
            registers.af, registers.bc, registers.de, registers.hl, registers.sp
        )
    );
}

#[test]
fn trace_keeps_only_the_most_recent_instructions() {
    let mut emulator = emulator();
    emulator.set_trace_capacity(2);
    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    let pcs: Vec<u16> = emulator.trace().entries().map(|e| e.pc).collect();
    assert_eq!(pcs, [0x0158, 0x0159]);

    emulator.set_trace_capacity(0);
    emulator.run();
    assert!(emulator.trace().is_empty());
}

#[test]
fn registers_can_be_written() {
    let mut emulator = emulator();

    // jump straight to `ld [$C000], a` with a value of our own in A
    emulator.write_pc(0x0152);
    emulator.write_a(0x99);
    emulator.run();
    assert_eq!(emulator.read_memory(0xC000), 0x99);

    emulator.write_af(0x12FF);
    emulator.write_bc(0x3456);
    emulator.write_de(0x789A);
    emulator.write_hl(0xBCDE);
    emulator.write_sp(0xDFF0);
    assert_eq!(
        emulator.registers(),
        Registers {
            // the low nibble of F does not exist
            af: 0x12F0,
            bc: 0x3456,
            de: 0x789A,
            hl: 0xBCDE,
            sp: 0xDFF0,
            pc: 0x0155,
        }
    );

    emulator.write_f(0x0F);
    emulator.write_b(0x01);
    emulator.write_c(0x02);
    emulator.write_d(0x03);
    emulator.write_e(0x04);
    emulator.write_h(0x05);
    emulator.write_l(0x06);
    assert_eq!(emulator.read_af(), 0x1200);
    assert_eq!(emulator.read_bc(), 0x0102);
    assert_eq!(emulator.read_de(), 0x0304);
    assert_eq!(emulator.read_hl(), 0x0506);
}

#[test]
fn ime_and_halted_follow_the_cpu() {
    let mut emulator = emulator();
    emulator.write_pc(FLAGS);
    assert!(!emulator.ime());

    // ei takes effect after the instruction following it
    emulator.run();
    assert!(!emulator.ime());
    emulator.run();
    assert!(emulator.ime());

    // di
    emulator.run();
    assert!(!emulator.ime());

    // halt, with no interrupt pending
    assert!(!emulator.halted());
    emulator.run();
    assert!(emulator.halted());
    assert!(!emulator.halt_bug());
}

#[test]
fn halt_bug_is_visible_and_its_lookahead_is_not_a_data_read() {
    let recorder = Recorder::default();
    let events = Rc::clone(&recorder.events);
    let mut emulator = emulator();
    emulator.write_pc(HALT_BUG);
    emulator.add_checker(Box::new(recorder));

    assert_eq!(
        emulator.run_until_pc(HALT_BUG_INC_B, 100),
        RunOutcome::Reached
    );
    assert!(emulator.halt_bug());
    assert!(!emulator.halted());
    assert_eq!(
        events.borrow().last(),
        Some(&Event::Read(HALT_BUG_INC_B, 0x04, ReadKind::Operand))
    );

    // `inc b` runs twice
    assert_eq!(
        emulator.run_until_pc(HALT_BUG_DONE, 100),
        RunOutcome::Reached
    );
    assert!(!emulator.halt_bug());
    assert_eq!(emulator.read_b(), 2);
    let executed: Vec<Event> = events
        .borrow()
        .iter()
        .copied()
        .filter(|event| matches!(event, Event::Exec(HALT_BUG_INC_B, _)))
        .collect();
    assert_eq!(executed, [Event::Exec(HALT_BUG_INC_B, 0x04); 2]);
}

#[test]
fn ppu_blocking_changes_over_a_frame() {
    let mut emulator = emulator();
    assert_eq!(emulator.run_until_pc(EVENTS_DONE, 100), RunOutcome::Reached);

    // the LCD is on after boot, so spinning for a frame passes through every PPU mode
    let mut seen = [[false; 2]; 4];
    for _ in 0..10_000 {
        emulator.run();
        let blocked = [
            emulator.oam_read_blocked(),
            emulator.oam_write_blocked(),
            emulator.vram_read_blocked(),
            emulator.vram_write_blocked(),
        ];
        for (seen, blocked) in seen.iter_mut().zip(blocked) {
            seen[usize::from(blocked)] = true;
        }
    }
    assert_eq!(seen, [[true; 2]; 4]);
}

#[test]
fn on_exec_end_follows_each_executed_instruction() {
    /// What an `on_exec_end` saw: the instruction, the `A` register, and whether the CPU was
    /// halted by then.
    type End = (u16, u8, u8, bool);

    #[derive(Default)]
    struct Ends {
        ends: Rc<RefCell<Vec<End>>>,
    }
    impl Checker for Ends {
        fn name(&self) -> &'static str {
            "ends"
        }

        fn on_exec_end(&mut self, cx: &mut Context<'_>, pc: u16, opcode: u8) {
            assert_eq!(cx.pc(), pc);
            let a = cx.registers().af.to_be_bytes()[0];
            self.ends.borrow_mut().push((pc, opcode, a, cx.halted()));
        }
    }

    let checker = Ends::default();
    let ends = Rc::clone(&checker.ends);
    let mut emulator = emulator();
    emulator.add_checker(Box::new(checker));

    // the registers are the ones the instruction left behind
    emulator.write_a(0x00);
    emulator.write_pc(EVENTS);
    emulator.run();
    assert_eq!(*ends.borrow(), [(EVENTS, 0x3E, 0x42, false)]);
    ends.borrow_mut().clear();

    // `ei`, `nop`, `di`, `halt`, and then steps that execute nothing
    emulator.write_pc(FLAGS);
    emulator.run_for(10);
    assert_eq!(
        *ends.borrow(),
        [
            (FLAGS, 0xFB, 0x42, false),
            (FLAGS + 1, 0x00, 0x42, false),
            (FLAGS + 2, 0xF3, 0x42, false),
            (FLAGS + 3, 0x76, 0x42, true),
        ]
    );
}
