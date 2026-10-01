use super::InstrCtx;
use crate::lir::{BlockId, CarryId, Operand8, ZeroId};
use crate::mir;
use crate::ty::Ty;

impl InstrCtx<'_> {
    pub(super) fn eq_from_comparison_op(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
    ) -> (ZeroId, mir::Condition) {
        match self.function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (z, _c) = self.cp_from_r8(left, right, block);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(self.ty_registry) == 8 => {
                let (z, _c) = self.cp_from_r8(left, right, block);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(self.ty_registry) == 16 => {
                let z = self.eq_from_r16(left, right, block);
                (z, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    /// Lowers 16-bit equality to `(left_low ^ right_low) | (left_high ^ right_high)`, whose Z flag
    /// is set iff both bytes are equal. `lt_from_r16`'s `SUB`/`SBC` can't be used here since `SBC`
    /// only sets Z from the high byte's result. Signedness doesn't affect equality. A zero literal
    /// byte skips its `XOR`, so `x == 0` lowers to `LD A,high; OR A,low`.
    pub(super) fn eq_from_r16(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
    ) -> ZeroId {
        let (right_high, right_low) = self.r16_to_operand(right, block);
        let (left_high, left_low) = self.locals.r16s[left];

        let low_diff = if matches!(right_low, Operand8::Literal(0)) {
            left_low
        } else {
            let a = self.lir.insert_ld_a(block, left_low);
            let (_z, _c) = self.lir.insert_xor(block, a, right_low);
            self.lir.insert_move_a(block, a)
        };

        let a = self.lir.insert_ld_a(block, left_high);
        if !matches!(right_high, Operand8::Literal(0)) {
            let (_z, _c) = self.lir.insert_xor(block, a, right_high);
        }
        let (z, _c) = self.lir.insert_or(block, a, Operand8::Reg(low_diff));
        z
    }

    pub(super) fn lt_from_comparison_op(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
    ) -> (CarryId, mir::Condition) {
        match self.function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (_z, c) = self.cp_from_r8(left, right, block);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(self.ty_registry) == 8 => {
                let (_z, c) = self.cp_from_r8(left, right, block);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(self.ty_registry) == 16 => {
                let c = self.lt_from_r16(left, right, block);
                (c, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    pub(super) fn cp_from_r8(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
    ) -> (ZeroId, CarryId) {
        match self.function.locals[left].ty {
            Ty::U1 => {
                let (mut right, right_condition) = self.r1_to_operand(right, block);
                if right_condition != mir::Condition::NotZero {
                    right = match right {
                        Operand8::Reg(right) => {
                            let a = self.lir.insert_ld_a(block, right);
                            self.lir.insert_cpl(block, a);
                            Operand8::Reg(self.lir.insert_move_a(block, a))
                        }
                        Operand8::Literal(value) => Operand8::Literal(!value),
                        Operand8::Ptr(_) => {
                            unimplemented!("r1_to_operand doesn't produce Operand8::Ptr")
                        }
                    };
                }
                let (left, left_condition) = self.locals.r1s[left];
                let left = self.r1_to_r8(left, block);
                let a = self.lir.insert_ld_a(block, left);
                if left_condition != mir::Condition::NotZero {
                    self.lir.insert_cpl(block, a);
                }
                self.lir.insert_cp(block, a, right)
            }
            Ty::U2 => todo!("{left:?} {right:?} u2"),
            Ty::U4 => todo!("{left:?} {right:?} u4"),
            Ty::I8 => {
                // for I8, we need to take into account that negative numbers have their high bit
                // set, so we need to XOR both operands so that the high bit is inverted and we can
                // use a regular comparison
                let right = self
                    .lir
                    .insert_ld8_operand(block, self.r8_to_operand(right));

                let a = self.lir.insert_ld_a(block, right);
                let (_z, _c) = self.lir.insert_xor(block, a, Operand8::Literal(0x80));
                let right = self.lir.insert_move_a(block, a);

                let a = self.lir.insert_ld_a(block, self.locals.r8s[left]);
                let (_z, _c) = self.lir.insert_xor(block, a, Operand8::Literal(0x80));

                self.lir.insert_cp(block, a, Operand8::Reg(right))
            }
            ty if ty.bit_size(self.ty_registry) == 8 => {
                let right = self.r8_to_operand(right);
                let a = self.lir.insert_ld_a(block, self.locals.r8s[left]);
                self.lir.insert_cp(block, a, right)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    pub(super) fn lt_from_r16(
        &mut self,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
    ) -> CarryId {
        let is_signed = self.function.locals[left].ty.is_signed_int();
        let (mut right_high, right_low) = self.r16_to_operand(right, block);
        if is_signed {
            right_high = match right_high {
                Operand8::Reg(right_high) => {
                    let a = self.lir.insert_ld_a(block, right_high);
                    let (_z, _c) = self
                        .lir
                        .insert_xor(block, a, Operand8::Literal(0b1000_0000));
                    Operand8::Reg(self.lir.insert_move_a(block, a))
                }
                Operand8::Literal(value) => Operand8::Literal(value ^ 0b1000_0000),
                Operand8::Ptr(_) => unimplemented!("r16_to_operand doesn't produce Operand8::Ptr"),
            };
        }
        let (mut left_high, left_low) = self.locals.r16s[left];
        if is_signed {
            let a = self.lir.insert_ld_a(block, left_high);
            let (_z, _c) = self
                .lir
                .insert_xor(block, a, Operand8::Literal(0b1000_0000));
            left_high = self.lir.insert_move_a(block, a);
        }

        let a = self.lir.insert_ld_a(block, left_low);
        let (_z, c_in) = self.lir.insert_sub(block, a, right_low);

        let a = self.lir.insert_ld_a(block, left_high);
        let (_z, c) = self.lir.insert_sbc(block, a, right_high, c_in);
        c
    }
}
