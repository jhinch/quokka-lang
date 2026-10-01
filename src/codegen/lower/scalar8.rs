use super::InstrCtx;
use crate::lir::{BlockId, Operand8, R8Id};
use crate::mir;
use crate::num::u3;

impl InstrCtx<'_> {
    #[must_use]
    pub(super) fn generate_from_scalar8_instr(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
    ) -> Option<(mir::LocalId, R8Id)> {
        Some(match instruction {
            mir::Instr::Nop => return None,
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = self.lir.insert_ld8_value(block, 0);
                (*target, r8)
            }
            mir::Instr::ReturnValue { target } => {
                let r8 = self.lir.insert_ld_return_8(block);
                let r8 = self.lir.insert_move_a(block, r8);
                (*target, r8)
            }
            mir::Instr::Phi { target, sources: _ } => (*target, self.locals.phi_r8s[target].0),
            mir::Instr::Load {
                target,
                source,
                projections,
            } => {
                let (addr, bit_offset) =
                    self.projected_global_to_addr(*source, projections.as_slice(), block);
                assert_eq!(bit_offset, u3::new(0), "misaligned 8-bit ty");
                let r8 = self.lir.insert_ld8_hl_ptr(block, addr);
                (*target, r8)
            }
            mir::Instr::Store {
                target,
                projections,
                source,
            } => {
                let (target, bit_offset) =
                    self.projected_global_to_addr(*target, projections.as_slice(), block);
                assert_eq!(bit_offset, u3::new(0), "misaligned 8-bit ty");
                match self.r8_to_operand(source) {
                    Operand8::Reg(source) => {
                        self.lir.insert_store_hl(block, target, source);
                    }
                    Operand8::Literal(source) => {
                        self.lir.insert_store_value(block, target, source);
                    }
                    unexpected => unreachable!(
                        "projected_global_to_addr and r8_to_operand should not produce {unexpected:?}"
                    ),
                }
                return None;
            }
            mir::Instr::Copy { target, source } => {
                let r8 = self
                    .lir
                    .insert_ld8_operand(block, self.r8_to_operand(source));
                (*target, r8)
            }
            mir::Instr::Read {
                target,
                source,
                projections,
            } => {
                let (r8, u3) = self.projected_local_to_r8(*source, projections.as_slice());
                assert_eq!(u3, u3::new(0), "misaligned 8-bit ty");
                (*target, r8)
            }
            mir::Instr::Write { .. } => todo!("{instruction:?} u8"),
            mir::Instr::BitCast { target, source } => (*target, self.locals.r8s[*source]),
            mir::Instr::Not { target, source } => {
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*source]);
                self.lir.insert_cpl(block, a);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Neg { target, source } => {
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*source]);
                self.lir.insert_cpl(block, a);
                let r8 = self.lir.insert_move_a(block, a);
                let _z = self.lir.insert_inc(block, r8);
                (*target, r8)
            }
            mir::Instr::RotateLeft { target, source } => {
                let r8 = self.lir.insert_ld8(block, self.locals.r8s[*source]);
                let (_z, _c) = self.lir.insert_rlc(block, r8);
                (*target, r8)
            }
            mir::Instr::RotateRight { target, source } => {
                let r8 = self.lir.insert_ld8(block, self.locals.r8s[*source]);
                let (_z, _c) = self.lir.insert_rrc(block, r8);
                (*target, r8)
            }
            mir::Instr::ShiftLeft { target, source } => {
                let r8 = self.lir.insert_ld8(block, self.locals.r8s[*source]);
                let (_z, _c) = self.lir.insert_sla(block, r8);
                (*target, r8)
            }
            mir::Instr::ShiftRight { target, source } => {
                let r8 = self.lir.insert_ld8(block, self.locals.r8s[*source]);
                let (_z, _c) = if self.function.locals[*target].ty.is_signed_int() {
                    self.lir.insert_sra(block, r8)
                } else {
                    self.lir.insert_srl(block, r8)
                };
                (*target, r8)
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Ge { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*left]);
                let (_z, _c) = self.lir.insert_and(block, a, right);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*left]);
                let (_z, _c) = self.lir.insert_or(block, a, right);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*left]);
                let (_z, _c) = self.lir.insert_xor(block, a, right);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*left]);
                let (_z, _c) = self.lir.insert_add(block, a, right);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[*left]);
                let (_z, _c) = self.lir.insert_sub(block, a, right);
                let r8 = self.lir.insert_move_a(block, a);
                (*target, r8)
            }
            mir::Instr::Transfer { .. } | mir::Instr::Fill { .. } => {
                unimplemented!("transfer/fill handled separately")
            }
            mir::Instr::Breakpoint => unimplemented!("breakpoint handled separately"),
        })
    }

    pub(super) fn r8_to_operand(&self, right: &mir::Operand) -> Operand8 {
        match right {
            mir::Operand::Local(right) => Operand8::Reg(self.locals.r8s[*right]),
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => Operand8::Literal(right.value as u8),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 8-bit instruction using fn ptr")
            }
        }
    }
}
