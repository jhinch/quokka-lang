use super::LoweredGlobal;
use crate::lir::{BlockId, CarryId, CodeSectionBuilder, FnAddr, R8Id, ZeroId};
use crate::mir;
use crate::num::u3;
use crate::ty::{Ty, TyRegistry};
use indexmap::IndexMap;
use indexmap::map::Entry;
use select::{Plan, Selection};
use slotmap::{SecondaryMap, SparseSecondaryMap};

mod alu;
mod bulk;
mod compare;
mod place;
mod scalar16;
mod scalar8;
mod select;
mod terminator;
mod u1;

/// Everything needed to lower one MIR function to LIR. Built once per function.
pub(super) struct InstrCtx<'a> {
    ty_registry: &'a TyRegistry,
    mir: &'a mir::Mir,
    function: &'a mir::Function,
    globals: &'a SecondaryMap<mir::GlobalId, LoweredGlobal>,
    functions: &'a SecondaryMap<mir::FunctionId, FnAddr>,
    blocks: SecondaryMap<mir::BlockId, BlockId>,
    lir: &'a mut CodeSectionBuilder,
    locals: Locals,
}

impl<'a> InstrCtx<'a> {
    pub(super) fn new(
        ty_registry: &'a TyRegistry,
        mir: &'a mir::Mir,
        function: &'a mir::Function,
        start_block: BlockId,
        globals: &'a SecondaryMap<mir::GlobalId, LoweredGlobal>,
        functions: &'a SecondaryMap<mir::FunctionId, FnAddr>,
        lir: &'a mut CodeSectionBuilder,
    ) -> Self {
        let mut blocks = SecondaryMap::new();
        blocks.insert(function.start_block, start_block);
        for mir_block in function.traverse_blocks() {
            blocks.insert(
                mir_block,
                if function.start_block == mir_block {
                    start_block
                } else {
                    lir.new_block()
                },
            );
        }
        InstrCtx {
            ty_registry,
            mir,
            function,
            globals,
            functions,
            blocks,
            lir,
            locals: Locals::default(),
        }
    }

    pub(super) fn generate_from_function(mut self) {
        let function = self.function;
        for mir_block in function.traverse_blocks() {
            for instruction in &function.blocks[mir_block].instructions {
                if let mir::Instr::Phi { target, sources } = instruction {
                    let phi = match function.locals[*target]
                        .ty
                        .bit_size(self.ty_registry)
                        .into_u32()
                    {
                        1 => Value::Mask(self.lir.reserve_phi(), mir::Condition::NotZero),
                        8 => Value::Byte(self.lir.reserve_phi()),
                        16 => {
                            let low = self.lir.reserve_phi();
                            let high = self.lir.reserve_phi();
                            Value::Pair(high, low)
                        }
                        _ => unimplemented!(
                            "{} cannot fit in a 8-bit or 16-bit register",
                            function.locals[*target].ty.display(self.ty_registry)
                        ),
                    };
                    self.locals.phi_targets.insert(*target, (phi, Vec::new()));
                    for (mir_block, local) in sources {
                        let block = self.blocks[mir_block];
                        match self
                            .locals
                            .phi_sources
                            .entry(block)
                            .unwrap()
                            .or_default()
                            .entry(*local)
                        {
                            Entry::Occupied(_) => panic!(
                                "block {mir_block:?} already has phi registered for {local:?}"
                            ),
                            Entry::Vacant(entry) => {
                                entry.insert(phi);
                            }
                        }
                        self.locals.phi_targets[target].1.push(block);
                    }
                }
            }
        }
        let plan = Plan::new(function);
        for mir_block in function.traverse_blocks() {
            let block = self.blocks[mir_block];
            let instructions = &function.blocks[mir_block].instructions;
            for (instruction, selection) in instructions.iter().zip(plan.block(mir_block)) {
                self.generate_from_selection(*selection, instruction, block);
            }
            // `r1_to_r8` needs all of `self`, so the sources are copied out of `locals` first
            let phi_sources: Vec<(mir::LocalId, Value)> = self
                .locals
                .phi_sources
                .get(block)
                .into_iter()
                .flatten()
                .map(|(&local, &phi)| (local, phi))
                .collect();
            for (local, phi) in phi_sources {
                match phi {
                    Value::Mask(target, _) => {
                        let value = self.u1(local);
                        let mut source = self.r1_to_r8(value, block);
                        if value.condition() != mir::Condition::NotZero {
                            let a = self.lir.insert_ld_a(block, source);
                            self.lir.insert_cpl(block, a);
                            source = self.lir.insert_move_a(block, a);
                        }
                        self.lir.insert_phi_ld(block, target, source);
                    }
                    Value::Byte(target) => {
                        self.lir.insert_phi_ld(block, target, self.byte(local));
                    }
                    Value::Pair(target_high, target_low) => {
                        let (source_high, source_low) = self.pair(local);
                        self.lir.insert_phi_ld(block, target_low, source_low);
                        self.lir.insert_phi_ld(block, target_high, source_high);
                    }
                    Value::Zero(..) | Value::Carry(..) | Value::Bit(..) => {
                        unreachable!("phi targets are never lowered to {phi:?}")
                    }
                }
            }
            self.generate_from_terminator(&function.blocks[mir_block].terminator, block);
        }
        for (phi, blocks) in self.locals.phi_targets.values() {
            match *phi {
                Value::Mask(r8, _) | Value::Byte(r8) => {
                    self.lir.complete_phi(r8, blocks.as_slice());
                }
                Value::Pair(high, low) => {
                    self.lir.complete_phi(low, blocks.as_slice());
                    self.lir.complete_phi(high, blocks.as_slice());
                }
                Value::Zero(..) | Value::Carry(..) | Value::Bit(..) => {
                    unreachable!("phi targets are never lowered to {phi:?}")
                }
            }
        }
    }

    /// The lowered 1-bit value of `local`.
    fn u1(&self, local: mir::LocalId) -> Value {
        match self.locals.values[local] {
            value @ (Value::Zero(..) | Value::Carry(..) | Value::Mask(..) | Value::Bit(..)) => {
                value
            }
            value => panic!("{local:?} is lowered to {value:?}, expected a 1-bit value"),
        }
    }

    /// The register holding the byte-sized `local`.
    fn byte(&self, local: mir::LocalId) -> R8Id {
        match self.locals.values[local] {
            Value::Byte(r8) => r8,
            value => panic!("{local:?} is lowered to {value:?}, expected a byte"),
        }
    }

    /// The `(high, low)` registers holding the 16-bit `local`.
    fn pair(&self, local: mir::LocalId) -> (R8Id, R8Id) {
        match self.locals.values[local] {
            Value::Pair(high, low) => (high, low),
            value => panic!("{local:?} is lowered to {value:?}, expected a byte pair"),
        }
    }

    fn ty_from_instr(&self, instruction: &mir::Instr) -> Ty {
        match (instruction.target_local(), instruction) {
            (Some(target), _) => self.function.locals[target].ty,
            (
                None,
                mir::Instr::Store {
                    target,
                    projections,
                    ..
                },
            ) => self.projected_ty(*target, projections),
            (None, _) => unreachable!("{instruction:?}"),
        }
    }

    /// generate LIR for the MIR instruction as planned by its selection.
    fn generate_from_selection(
        &mut self,
        selection: Selection,
        instruction: &mir::Instr,
        block: BlockId,
    ) {
        match selection {
            Selection::Covered => {}
            Selection::Instr => match instruction {
                mir::Instr::Nop => {
                    // currently we don't translate Nop in MIR to Nop in LIR
                }
                mir::Instr::Breakpoint => self.lir.insert_breakpoint(block),
                mir::Instr::Transfer { .. } | mir::Instr::Fill { .. } => {
                    self.generate_from_transfer_or_fill(instruction, block);
                }
                _ => {
                    let ty = self.ty_from_instr(instruction);
                    self.generate_from_instr(instruction, ty, block);
                }
            },
            Selection::BitTest {
                target,
                source,
                bit,
                condition,
            } => {
                // no LIR is needed until the value is used: `r1_to_flag` lowers it to `BIT`
                let r8 = match self.locals.values[source] {
                    Value::Byte(r8) => r8,
                    Value::Pair(high, _low) if bit >= 8 => high,
                    Value::Pair(_high, low) => low,
                    value => panic!("{source:?} is lowered to {value:?}, expected a byte or pair"),
                };
                self.define(target, Value::Bit(r8, u3::new(bit % 8), condition), block);
            }
        }
    }

    /// generate LIR from the MIR instruction.
    ///
    /// An additional move after every lowered MIR instruction to ensure that between
    /// A MIR instruction will never have a pre-colored source as this would mean that there
    /// is likely interference due to the restrictive machine instructions.
    /// Unnecessary moves will be coalesced during register allocation if not needed.
    fn generate_from_instr(&mut self, instruction: &mir::Instr, ty: Ty, block: BlockId) {
        let lowered = match ty {
            Ty::U1 => self.generate_from_u1_instr(instruction, block),
            Ty::U2 => todo!("{instruction:?} {:?}", Ty::U2),
            Ty::U4 => todo!("{instruction:?} {:?}", Ty::U4),
            Ty::U8 | Ty::I8 => self.generate_from_scalar8_instr(instruction, block),
            Ty::U16 | Ty::I16 => self.generate_from_scalar16_instr(instruction, block),
            ty if ty.is_struct() || ty.is_array() || ty.is_fn_ptr() => match instruction {
                mir::Instr::Alloc { .. }
                | mir::Instr::ReturnValue { .. }
                | mir::Instr::Phi { .. }
                | mir::Instr::Load { .. }
                | mir::Instr::Store { .. }
                | mir::Instr::Copy { .. }
                | mir::Instr::BitCast { .. }
                | mir::Instr::Read { .. }
                | mir::Instr::Write { .. }
                | mir::Instr::Eq { .. }
                | mir::Instr::Ne { .. } => match ty.bit_size(self.ty_registry).into_u32() {
                    8 => self.generate_from_scalar8_instr(instruction, block),
                    16 => self.generate_from_scalar16_instr(instruction, block),
                    _ => unimplemented!(
                        "Valid MIR should never produce structs, arrays or fn ptrs which are not 8 or 16-bit"
                    ),
                },
                _ => unimplemented!(
                    "Only alloc, equality, reads and writes are supported on structs, arrays and fn ptrs"
                ),
            },
            ty => unimplemented!("{ty:?}"),
        };
        if let Some((target, value)) = lowered {
            self.define(target, value, block);
        }
    }

    /// Records `value` as the lowered value of `target`, first moving it into the return
    /// registers if `target` is the return local.
    fn define(&mut self, target: mir::LocalId, mut value: Value, block: BlockId) {
        if self.function.locals[target].is_return {
            value = self.move_to_return_registers(value, block);
        }
        self.locals.values.insert(target, value);
    }

    /// Moves `value` into the registers of the return convention: `A` for 1-bit and byte values,
    /// `BC` for byte pairs.
    fn move_to_return_registers(&mut self, value: Value, block: BlockId) -> Value {
        match value {
            Value::Zero(..) | Value::Carry(..) | Value::Mask(..) | Value::Bit(..) => {
                let r8 = self.r1_to_r8(value, block);
                let a = self.lir.insert_ld_a(block, r8);
                if value.condition() != mir::Condition::NotZero {
                    self.lir.insert_cpl(block, a);
                }
                Value::Mask(a.upcast_to_r8_without_move(), mir::Condition::NotZero)
            }
            Value::Byte(r8) => {
                let a = self.lir.insert_ld_a(block, r8);
                Value::Byte(a.upcast_to_r8_without_move())
            }
            Value::Pair(high, low) => {
                let (b, c) = self.lir.insert_ld_bc(block, high, low);
                Value::Pair(b, c)
            }
        }
    }
}

/// The lowered values of a function's locals, and the phi registers they feed.
#[derive(Default)]
struct Locals {
    values: SparseSecondaryMap<mir::LocalId, Value>,
    /// For each block, the phi target each of its locals must be copied into before the terminator.
    phi_sources: SparseSecondaryMap<BlockId, IndexMap<mir::LocalId, Value>>,
    phi_targets: IndexMap<mir::LocalId, (Value, Vec<BlockId>)>,
}

/// How a MIR local is represented in LIR. The 1-bit variants carry the `mir::Condition` under
/// which the value is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    /// 1-bit value held in the zero flag
    Zero(ZeroId, mir::Condition),
    /// 1-bit value held in the carry flag
    Carry(CarryId, mir::Condition),
    /// 1-bit value held in a register, with 0xFF as 1 and 0x00 as 0
    Mask(R8Id, mir::Condition),
    /// 1-bit value held in a single bit of a register
    Bit(R8Id, u3, mir::Condition),
    /// 8-bit value
    Byte(R8Id),
    /// 16-bit value as `(high, low)`
    Pair(R8Id, R8Id),
}

impl Value {
    /// The polarity of a 1-bit value.
    fn condition(self) -> mir::Condition {
        match self {
            Value::Zero(_, condition)
            | Value::Carry(_, condition)
            | Value::Mask(_, condition)
            | Value::Bit(_, _, condition) => condition,
            Value::Byte(_) | Value::Pair(..) => panic!("{self:?} is not a 1-bit value"),
        }
    }

    /// The same 1-bit representation with a different polarity.
    fn with_condition(self, condition: mir::Condition) -> Value {
        match self {
            Value::Zero(z, _) => Value::Zero(z, condition),
            Value::Carry(c, _) => Value::Carry(c, condition),
            Value::Mask(r8, _) => Value::Mask(r8, condition),
            Value::Bit(r8, bit, _) => Value::Bit(r8, bit, condition),
            Value::Byte(_) | Value::Pair(..) => panic!("{self:?} is not a 1-bit value"),
        }
    }
}
