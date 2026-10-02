//! Moves the constant operand of a commutative instruction to the right, so the other rules and
//! codegen each only have to recognize one shape, e.g. `4 & x` becomes `x & 4`.
//!
//! - **Constant on the left:** with `c` a constant, `c + x`, `c & x`, `c | x` and `c ^ x` swap
//!   their operands. The constant becomes a literal operand of the instruction.
//!
//! Only the instruction itself is rewritten. The `Copy` that defined the constant is left for
//! dead code elimination if that was its only use. The target holds the same value as before, so
//! none of this is restricted by breakpoints.

use super::{Context, Rewrite, Rule, to_literal};
use crate::mir::{BinOp, BlockId, Operand};

pub(super) struct Commute;

impl Rule for Commute {
    fn name(&self) -> &'static str {
        "commute"
    }

    fn rewrites(&self, ctx: &mut Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        let Some(binop) = ctx.function.blocks[position.0].instructions[position.1].as_binop()
        else {
            return vec![];
        };
        if !binop.op.is_commutative() {
            return vec![];
        }
        let ty = ctx.operand_ty(&binop);
        let Some(constant) = ctx.constant(&Operand::Local(binop.left), ty) else {
            return vec![];
        };
        // Two constants are left for constant propagation to fold.
        let Operand::Local(x) = binop.right else {
            return vec![];
        };
        if ctx.constant(&binop.right, ty).is_some() {
            return vec![];
        }
        vec![Rewrite::replace(
            position,
            BinOp {
                op: binop.op,
                target: binop.target,
                left: x,
                right: to_literal(constant),
            }
            .into_instr(),
        )]
    }
}
