//! A ring buffer of the most recently executed instructions.

use crate::check::Registers;
use std::collections::VecDeque;
use std::fmt;

/// One executed instruction: where it was, its encoded bytes, and the registers going in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEntry {
    /// The address of the instruction.
    pub pc: u16,
    /// The registers before the instruction executed.
    pub registers: Registers,
    bytes: [u8; 3],
    len: u8,
}

impl TraceEntry {
    /// The instruction's encoded bytes, opcode first.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// The instruction's opcode.
    #[must_use]
    pub fn opcode(&self) -> u8 {
        self.bytes[0]
    }
}

impl fmt::Display for TraceEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self
            .bytes()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        write!(f, "${:04X}  {bytes:<8}  {}", self.pc, self.registers)
    }
}

/// The last instructions the emulator executed, oldest first.
#[derive(Debug, Clone)]
pub struct Trace {
    entries: VecDeque<TraceEntry>,
    capacity: usize,
}

impl Trace {
    /// How many instructions a trace holds unless told otherwise.
    pub const DEFAULT_CAPACITY: usize = 32;

    pub(crate) fn new(capacity: usize) -> Self {
        Trace {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// The recorded instructions, oldest first.
    #[must_use]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &TraceEntry> {
        self.entries.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.entries.len() > capacity {
            self.entries.pop_front();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn push(&mut self, pc: u16, opcode: u8, registers: Registers) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(TraceEntry {
            pc,
            registers,
            bytes: [opcode, 0, 0],
            len: 1,
        });
    }

    /// Append an operand byte to the most recently pushed instruction.
    pub(crate) fn push_operand(&mut self, byte: u8) {
        if let Some(entry) = self.entries.back_mut()
            && let Some(slot) = entry.bytes.get_mut(usize::from(entry.len))
        {
            *slot = byte;
            entry.len += 1;
        }
    }
}

impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for entry in &self.entries {
            writeln!(f, "{entry}")?;
        }
        Ok(())
    }
}
