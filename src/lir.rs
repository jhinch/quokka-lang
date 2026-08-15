//! # Quokka Lang's Low Intermediate Representation (LIR)
//!
//! LIR ([`Lir`]) represents a Quokka program as a collection of control flow
//! graphs with infinite virtual registers with all Phi instructions eliminated.
//! The instructions ([`Instr`]) within a basic block ([`Block`]) represent concrete
//! machine level instructions unlike [`crate::mir::Mir`] which represents language
//! level instructions.
//!
//! Virtual registers in LIR can be colored and allocated to physical registers to
//! produce the final assembly code.

pub struct Lir;

pub struct Block;

pub struct Instr;
