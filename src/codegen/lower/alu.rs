use super::InstrCtx;
use crate::lir::{BlockId, CarryId, Operand8, R8Id};
use crate::mir;

impl InstrCtx<'_> {
    /// Lowers one byte of `left op right` through the accumulator, returning the result and its
    /// carry out. With a `carry_in` the carry-in form of `op` is used (`adc` for `add`, `sbc` for
    /// `sub`); the bitwise ops have no carry and use the same form for every byte.
    pub(super) fn alu_byte(
        &mut self,
        block: BlockId,
        op: mir::OpKind,
        left: R8Id,
        right: Operand8,
        carry_in: Option<CarryId>,
    ) -> (R8Id, CarryId) {
        let a = self.lir.insert_ld_a(block, left);
        let (_z, c) = match (op, carry_in) {
            (mir::OpKind::Add, None) => self.lir.insert_add(block, a, right),
            (mir::OpKind::Add, Some(c_in)) => self.lir.insert_adc(block, a, right, c_in),
            (mir::OpKind::Sub, None) => self.lir.insert_sub(block, a, right),
            (mir::OpKind::Sub, Some(c_in)) => self.lir.insert_sbc(block, a, right, c_in),
            (mir::OpKind::BitAnd, _) => self.lir.insert_and(block, a, right),
            (mir::OpKind::BitOr, _) => self.lir.insert_or(block, a, right),
            (mir::OpKind::BitXor, _) => self.lir.insert_xor(block, a, right),
        };
        (self.lir.insert_move_a(block, a), c)
    }

    /// Lowers `left op right` on `(high, low)` byte pairs: the low byte first, then the high byte
    /// with the low byte's carry.
    pub(super) fn alu_pair(
        &mut self,
        block: BlockId,
        op: mir::OpKind,
        (left_high, left_low): (R8Id, R8Id),
        (right_high, right_low): (Operand8, Operand8),
    ) -> (R8Id, R8Id) {
        let (low, c) = self.alu_byte(block, op, left_low, right_low, None);
        let (high, _c) = self.alu_byte(block, op, left_high, right_high, Some(c));
        (high, low)
    }
}
