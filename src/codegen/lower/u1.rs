use super::{InstrCtx, R1};
use crate::lir::{BlockId, FlagId, Operand8, R8Id};
use crate::mir;

impl InstrCtx<'_> {
    #[must_use]
    pub(super) fn generate_from_u1_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
    ) -> Option<(mir::LocalId, R1, mir::Condition)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = self.lir.insert_ld8_value(block, 0);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::ReturnValue { target } => {
                let r8 = self.lir.insert_ld_return_8(block);
                let r8 = self.lir.insert_move_a(block, r8);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::Phi { target, .. } => (
                *target,
                R1::R8(self.locals.phi_r1s[target].0),
                mir::Condition::NotZero,
            ),
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit) =
                    self.projected_global_to_addr(*source, projections.as_slice(), block);
                let r8 = self.lir.insert_ld8_hl_ptr(block, addr);
                (*target, R1::Bit(r8, bit), mir::Condition::NotZero)
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (addr, bit) =
                    self.projected_global_to_addr(*target, projections.as_slice(), block);
                match self.r1_to_operand(source, block) {
                    (Operand8::Reg(source), condition) => {
                        let mask = 1 << u8::from(bit);
                        let existing = self.lir.insert_ld8_hl_ptr(block, addr);
                        let a = self.lir.insert_ld_a(block, existing);
                        let (_z, _c) = self.lir.insert_and(block, a, Operand8::Literal(!mask));
                        // `source` is a full-byte 0x00/0xFF boolean, not just the target bit, so
                        // we need to move the cleared existing byte out of the accumulator and
                        // mask `source` down to that single bit before merging the two
                        let existing = self.lir.insert_move_a(block, a);
                        let a = self.lir.insert_ld_a(block, source);
                        if condition != mir::Condition::NotZero {
                            self.lir.insert_cpl(block, a);
                        }
                        let (_z, _c) = self.lir.insert_and(block, a, Operand8::Literal(mask));
                        let (_z, _c) = self.lir.insert_or(block, a, Operand8::Reg(existing));
                        let (addr, _bit) =
                            self.projected_global_to_addr(*target, projections.as_slice(), block);
                        // this move is unnecessary, but we'll rely on register coalescing
                        // to fix it
                        let source = self.lir.insert_move_a(block, a);
                        self.lir.insert_store_hl(block, addr, source);
                    }
                    (Operand8::Literal(mut source), condition) => {
                        if condition != mir::Condition::NotZero {
                            source = !source;
                        }
                        if source != 0 {
                            self.lir.insert_set_ptr(block, bit, addr);
                        } else {
                            self.lir.insert_res_ptr(block, bit, addr);
                        }
                    }
                    unexpected => unreachable!(
                        "projected_global_to_addr and r1_to_operand should not produce {unexpected:?}"
                    ),
                }
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let (r8, condition) = match source {
                    mir::Operand::Local(source) => {
                        let (r1, condition) = self.locals.r1s[*source];
                        let r8 = self.r1_to_r8(r1, block);
                        (r8, condition)
                    }
                    mir::Operand::Literal(source) => {
                        let value = if source.value == 0 { 0x00u8 } else { 0xffu8 };
                        let r8 = self.lir.insert_ld8_value(block, value);
                        (r8, mir::Condition::NotZero)
                    }
                    mir::Operand::FnPtr(_) => unimplemented!("Cannot copy a fn ptr to a r1"),
                };
                (*target, R1::R8(r8), condition)
            }
            mir::Instr::Read {
                target,
                source,
                projections,
            } => {
                let (r8, bit) = self.projected_local_to_r8(*source, projections.as_slice());
                (*target, R1::Bit(r8, bit), mir::Condition::NotZero)
            }
            mir::Instr::Write { .. } => {
                unimplemented!("{instruction:?} should not be produced for u1 target")
            }
            mir::Instr::BitCast { target, source } => {
                let (flag, condition) = self.locals.r1s[*source];
                (*target, flag, condition)
            }
            mir::Instr::Not { target, source } => {
                let (flag, condition) = self.locals.r1s[*source];
                (*target, flag, !condition)
            }
            mir::Instr::Neg { .. } => unimplemented!("neg unsupported on u1"),
            mir::Instr::RotateLeft { target, source }
            | mir::Instr::RotateRight { target, source } => {
                let (r1, condition) = self.locals.r1s[*source];
                let r8 = self.r1_to_r8(r1, block);
                (*target, R1::R8(r8), condition)
            }
            mir::Instr::ShiftLeft { target, .. } | mir::Instr::ShiftRight { target, .. } => {
                let r8 = self.lir.insert_ld8_value(block, 0);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::Eq {
                target,
                left,
                right,
            } => {
                let (z, condition) = self.eq_from_comparison_op(*left, right, block);
                let z = self.lir.insert_move_zero(block, z);
                (*target, R1::Zero(z), condition)
            }
            mir::Instr::Ne {
                target,
                left,
                right,
            } => {
                let (z, condition) = self.eq_from_comparison_op(*left, right, block);
                let z = self.lir.insert_move_zero(block, z);
                (*target, R1::Zero(z), !condition)
            }
            mir::Instr::Lt {
                target,
                left,
                right,
            } => {
                let (c, condition) = self.lt_from_comparison_op(*left, right, block);
                let c = self.lir.insert_move_carry(block, c);
                (*target, R1::Carry(c), condition)
            }
            mir::Instr::Ge {
                target,
                left,
                right,
            } => {
                let (c, condition) = self.lt_from_comparison_op(*left, right, block);
                let c = self.lir.insert_move_carry(block, c);
                (*target, R1::Carry(c), !condition)
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = self.locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block);
                let left = self.r1_to_r8(r1, block);
                let a = self.lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    self.lir.insert_cpl(block, a);
                }
                let (_z, _c) = if right_condition == mir::Condition::NotZero {
                    self.lir.insert_and(block, a, right)
                } else {
                    self.lir.insert_or(block, a, right)
                };

                let r8 = self.lir.insert_move_a(block, a);
                (*target, R1::R8(r8), right_condition)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = self.locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block);
                let left = self.r1_to_r8(r1, block);
                let a = self.lir.insert_ld_a(block, left);
                if left_condition != right_condition {
                    self.lir.insert_cpl(block, a);
                }
                let (_z, _c) = if right_condition == mir::Condition::NotZero {
                    self.lir.insert_or(block, a, right)
                } else {
                    self.lir.insert_and(block, a, right)
                };

                let r8 = self.lir.insert_move_a(block, a);
                (*target, R1::R8(r8), right_condition)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            }
            | mir::Instr::Add {
                target,
                left,
                right,
            }
            | mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (r1, left_condition) = self.locals.r1s[*left];
                let (right, right_condition) = self.r1_to_operand(right, block);
                let left = self.r1_to_r8(r1, block);
                let a = self.lir.insert_ld_a(block, left);
                let (_z, _c) = self.lir.insert_xor(block, a, right);
                if left_condition != right_condition {
                    self.lir.insert_cpl(block, a);
                }
                let r8 = self.lir.insert_move_a(block, a);
                (*target, R1::R8(r8), mir::Condition::NotZero)
            }
            mir::Instr::Transfer { .. } | mir::Instr::Fill { .. } => {
                unimplemented!("transfer/fill handled separately")
            }
            mir::Instr::Breakpoint => unimplemented!("breakpoint handled separately"),
        })
    }

    pub(super) fn r1_to_flag(
        &mut self,
        r1: R1,
        condition: mir::Condition,
        block: BlockId,
    ) -> (FlagId, mir::Condition) {
        match r1 {
            R1::Zero(flag) => (FlagId::from(flag), condition),
            R1::Carry(flag) => (FlagId::from(flag), condition),
            R1::R8(r8) => {
                let r8 = self.lir.insert_ld_a(block, r8);
                let (z, _c) = self.lir.insert_cp(block, r8, Operand8::Literal(0xFF));
                (FlagId::from(z), condition)
            }
            R1::Bit(r8, bit) => {
                // `BIT` sets Z when the tested bit is 0, i.e. Z is SET for a *false* u1 value -
                // the opposite polarity to what the value represents so we need to invert the
                // condition
                (
                    FlagId::from(self.lir.insert_bit(block, bit, r8)),
                    !condition,
                )
            }
        }
    }

    pub(super) fn r1_to_r8(&mut self, r1: R1, block: BlockId) -> R8Id {
        match r1 {
            R1::Carry(c_in) => {
                let a = self.lir.insert_fresh_a(block);
                let (_z, _c) = self.lir.insert_sbc_a_a(block, a, c_in);

                self.lir.insert_move_a(block, a)
            }
            R1::Zero(z_in) => {
                // zero flags are difficult, unfortunately. We need to push them onto
                // the stack then pop onto a 16-bit register
                self.lir.insert_push_af_z(block, z_in);
                let r16 = self.lir.insert_pop_r16(block);
                let (_high, low) = self.lir.insert_split(block, r16);
                // low now has the zero flag stored in the 7th bit. We can now
                // use the rotate left strategy
                let (_z, c_in) = self.lir.insert_rlc(block, low);
                let a = self.lir.insert_fresh_a(block);
                let (_z, _c) = self.lir.insert_sbc_a_a(block, a, c_in);
                self.lir.insert_move_a(block, a)
            }
            R1::R8(r8) => r8,
            R1::Bit(r8, bit) => {
                match u8::from(bit) {
                    0 => {
                        // rotate right essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let r8 = self.lir.insert_ld8(block, r8);
                        let (_z, c_in) = self.lir.insert_rrc(block, r8);
                        let a = self.lir.insert_fresh_a(block);
                        let (_z, _c) = self.lir.insert_sbc_a_a(block, a, c_in);
                        self.lir.insert_move_a(block, a)
                    }
                    7 => {
                        // rotate left essentially moves the bit into the carry bit,
                        // allowing for handling it like the carry flag handling above
                        let r8 = self.lir.insert_ld8(block, r8);
                        let (_z, c_in) = self.lir.insert_rlc(block, r8);
                        let a = self.lir.insert_fresh_a(block);
                        let (_z, _c) = self.lir.insert_sbc_a_a(block, a, c_in);
                        self.lir.insert_move_a(block, a)
                    }
                    _ => {
                        let mask = 1 << u8::from(bit);
                        // AND with the mask sets all the undefined bits to 0
                        let a = self.lir.insert_ld_a(block, r8);
                        let (_z, _c) = self.lir.insert_and(block, a, Operand8::Literal(mask));
                        // ADD !mask + 1 will move the bit into the carry bit
                        let (_z, c_in) =
                            self.lir
                                .insert_add(block, a, Operand8::Literal((!mask) + 1));
                        // now we can use the same trick as before
                        let (_z, _c) = self.lir.insert_sbc_a_a(block, a, c_in);
                        self.lir.insert_move_a(block, a)
                    }
                }
            }
        }
    }

    pub(super) fn r1_to_operand(
        &mut self,
        right: &mir::Operand,
        block: BlockId,
    ) -> (Operand8, mir::Condition) {
        match right {
            mir::Operand::Local(right) => {
                let (r1, condition) = self.locals.r1s[*right];
                let r8 = self.r1_to_r8(r1, block);
                (Operand8::Reg(r8), condition)
            }
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => {
                let value = if right.value == 0 { 0x00u8 } else { 0xffu8 };
                (Operand8::Literal(value), mir::Condition::NotZero)
            }
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 1-bit instruction using fn ptr")
            }
        }
    }
}
