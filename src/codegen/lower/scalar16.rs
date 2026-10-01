use super::{InstrCtx, Value};
use crate::lir::{BlockId, Ld16Source, Operand8};
use crate::mir;
use crate::num::u3;

impl InstrCtx<'_> {
    #[must_use]
    pub(super) fn generate_from_scalar16_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
    ) -> Option<(mir::LocalId, Value)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let low = self.lir.insert_ld8_value(block, 0);
                let high = self.lir.insert_ld8_value(block, 0);
                (*target, Value::Pair(high, low))
            }
            mir::Instr::ReturnValue { target } => {
                let r16 = self.lir.insert_ld_return_16(block);
                let (high, low) = self.lir.insert_split(block, r16);
                let low = self.lir.insert_ld8(block, low);
                let high = self.lir.insert_ld8(block, high);
                (*target, Value::Pair(high, low))
            }
            mir::Instr::Phi { target, sources: _ } => (*target, self.locals.phi_targets[target].0),
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit_offset) =
                    self.projected_global_to_addr(*source, projections.as_slice(), block);
                assert_eq!(bit_offset, u3::new(0), "misaligned 16-bit ty");
                let low = self.lir.insert_ld8_inc(block, addr);
                let low = self.lir.insert_move_a(block, low);
                let high = self.lir.insert_ld8_hl_ptr(block, addr);
                (*target, Value::Pair(high, low))
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (high, low) = match source {
                    mir::Operand::Local(source) => self.pair(*source),
                    mir::Operand::FnPtr(source) => {
                        let fn_addr = self.functions[*source];
                        let r16 = self.lir.insert_ld16(block, Ld16Source::FnAddr(fn_addr));
                        self.lir.insert_split(block, r16)
                    }
                    mir::Operand::Literal(source) => {
                        #[allow(clippy::cast_possible_truncation)]
                        let value = source.value as u16;
                        let [low, high] = value.to_le_bytes();
                        let low = self.lir.insert_ld8_value(block, low);
                        let high = self.lir.insert_ld8_value(block, high);
                        (high, low)
                    }
                };
                let (addr, bit_offset) =
                    self.projected_global_to_addr(*target, projections.as_slice(), block);
                assert_eq!(bit_offset, u3::new(0), "misaligned 16-bit ty");
                let low = self.lir.insert_ld_a(block, low);
                self.lir.insert_store_inc(block, addr, low);
                self.lir.insert_store_hl(block, addr, high);
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let (source_high, source_low) = self.r16_to_operand(source, block);
                let target_high = self.lir.insert_ld8_operand(block, source_high);
                let target_low = self.lir.insert_ld8_operand(block, source_low);
                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::Read { .. } => {
                unimplemented!("{instruction:?} should not be produced for 16-bit target")
            }
            mir::Instr::Write {
                target,
                left,
                projections,
                right,
            } => {
                let bit_offset = self.projected_local_offset(*target, projections);
                let (source_high, source_low) = self.pair(*left);
                assert_eq!(
                    bit_offset.bits(),
                    u3::new(0),
                    "Currently narrow int writes are not supported for {instruction:?}"
                );
                match bit_offset.bytes() {
                    0 => {
                        // replace low byte, keep high byte
                        let target_low = self
                            .lir
                            .insert_ld8_operand(block, self.r8_to_operand(right));
                        let target_high = self.lir.insert_ld8(block, source_high);
                        (*target, Value::Pair(target_high, target_low))
                    }
                    1 => {
                        // replace high byte, keep low byte
                        let target_high = self
                            .lir
                            .insert_ld8_operand(block, self.r8_to_operand(right));
                        let target_low = self.lir.insert_ld8(block, source_low);
                        (*target, Value::Pair(target_high, target_low))
                    }
                    _ => unimplemented!("cannot support bit_offset {bit_offset} > 16-bit"),
                }
            }
            mir::Instr::BitCast { target, source } => {
                let (high, low) = self.pair(*source);
                (*target, Value::Pair(high, low))
            }
            mir::Instr::Not { target, source } => {
                let (source_high, source_low) = self.pair(*source);
                let a = self.lir.insert_ld_a(block, source_low);
                self.lir.insert_cpl(block, a);
                let target_low = self.lir.insert_move_a(block, a);
                let a = self.lir.insert_ld_a(block, source_high);
                self.lir.insert_cpl(block, a);
                let target_high = self.lir.insert_move_a(block, a);
                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::Neg { target, source } => {
                let (source_high, source_low) = self.pair(*source);
                let a = self.lir.insert_ld_a(block, source_low);
                self.lir.insert_cpl(block, a);
                let target_low = self.lir.insert_move_a(block, a);
                let a = self.lir.insert_ld_a(block, source_high);
                self.lir.insert_cpl(block, a);
                let target_high = self.lir.insert_move_a(block, a);
                let target16 = self.lir.insert_join(block, target_high, target_low);
                self.lir.insert_inc16(block, target16);
                let (target_high, target_low) = self.lir.insert_split(block, target16);
                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::RotateLeft { target, source } => {
                let (source_high, source_low) = self.pair(*source);

                let high_copy = self.lir.insert_ld8(block, source_high);
                let (_z, c) = self.lir.insert_sla(block, high_copy);

                let target_low = self.lir.insert_ld8(block, source_low);
                let (_z, c) = self.lir.insert_rl(block, target_low, c);

                let target_high = self.lir.insert_ld8(block, source_high);
                let (_z, _c) = self.lir.insert_rl(block, target_high, c);

                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::RotateRight { target, source } => {
                let (source_high, source_low) = self.pair(*source);

                let low_copy = self.lir.insert_ld8(block, source_low);
                let (_z, c) = self.lir.insert_sra(block, low_copy);

                let target_high = self.lir.insert_ld8(block, source_high);
                let (_z, c) = self.lir.insert_rr(block, target_high, c);

                let target_low = self.lir.insert_ld8(block, source_low);
                let (_z, _c) = self.lir.insert_rr(block, target_low, c);

                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (source_high, source_low) = self.pair(*source);

                let target_low = self.lir.insert_ld8(block, source_low);
                let (_z, c) = self.lir.insert_sla(block, target_low);

                let target_high = self.lir.insert_ld8(block, source_high);
                let (_z, _c) = self.lir.insert_rl(block, target_high, c);

                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::ShiftRight { target, source } => {
                let (source_high, source_low) = self.pair(*source);
                let target_high = self.lir.insert_ld8(block, source_high);
                let (_z, c) = if self.function.locals[*target].ty.is_signed_int() {
                    self.lir.insert_sra(block, target_high)
                } else {
                    self.lir.insert_srl(block, target_high)
                };
                let target_low = self.lir.insert_ld8(block, source_low);
                let (_z, _c) = self.lir.insert_rr(block, target_low, c);
                (*target, Value::Pair(target_high, target_low))
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Ge { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd { .. }
            | mir::Instr::BitOr { .. }
            | mir::Instr::BitXor { .. }
            | mir::Instr::Add { .. }
            | mir::Instr::Sub { .. } => {
                let binop = instruction
                    .as_binop()
                    .expect("every opcode of this arm is an OpKind");
                let right = self.r16_to_operand(&binop.right, block);
                let left = self.pair(binop.left);
                let (high, low) = self.alu_pair(block, binop.op, left, right);
                (binop.target, Value::Pair(high, low))
            }
            mir::Instr::Transfer { .. } | mir::Instr::Fill { .. } => {
                unimplemented!("transfer/fill handled separately")
            }
            mir::Instr::Breakpoint => unimplemented!("breakpoint handled separately"),
        })
    }

    /// Returns the `(high, low)` byte operands of a 16-bit operand. Literals stay as
    /// `Operand8::Literal` so byte-pair ops can use their `n8` immediate forms.
    pub(super) fn r16_to_operand(
        &mut self,
        right: &mir::Operand,
        block: BlockId,
    ) -> (Operand8, Operand8) {
        match right {
            mir::Operand::Local(right) => {
                let (high, low) = self.pair(*right);
                (Operand8::Reg(high), Operand8::Reg(low))
            }
            mir::Operand::Literal(right) => {
                #[allow(clippy::cast_possible_truncation)]
                let value = right.value as u16;
                let [low, high] = value.to_le_bytes();
                (Operand8::Literal(high), Operand8::Literal(low))
            }
            mir::Operand::FnPtr(function) => {
                let fn_addr = self.functions[*function];
                let r16 = self.lir.insert_ld16(block, Ld16Source::FnAddr(fn_addr));
                let (high, low) = self.lir.insert_split(block, r16);
                (Operand8::Reg(high), Operand8::Reg(low))
            }
        }
    }
}
