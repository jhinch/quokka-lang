//! Rewrites a signed ordered comparison against a constant as an unsigned one, e.g. for an `i8`
//! `x < 5` becomes `(x ^ 0x80) < 0x85` on the bits of `x` read as a `u8`.
//!
//! Flipping the sign bit of both operands maps the signed order onto the unsigned order. Codegen
//! would otherwise do that flip to both operands at runtime, constant included. Here the
//! constant's flip is folded at compile time, and a comparison with `0` needs no flip at all.
//!
//! With `SIGN` being only the sign bit of the operand type, `u` the bits of `x` cast to the
//! unsigned type of the same width, and `c` a constant:
//! - **Sign test:** `x < 0` becomes `u >= SIGN` and `x >= 0` becomes `u < SIGN`.
//! - **Bias:** `x < c` becomes `(u ^ SIGN) < (c ^ SIGN)`, likewise for `>=`.
//!
//! A constant on the left has already been moved to the right, and a comparison with the type's
//! minimum already folded, by the rule normalizing comparisons. A comparison of two non-constant
//! operands is left to codegen.
//!
//! The cast and the flip are new instructions defining new locals, inserted ahead of the
//! comparison. The target holds the same value as before, so none of this is restricted by
//! breakpoints.

use super::compare::{Comparison, Kind};
use super::{Context, Rewrite, Rule, to_literal};
use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{BlockId, Instr, Operand};
use crate::ty::Ty;

pub(super) struct SignedCompare;

impl Rule for SignedCompare {
    fn name(&self) -> &'static str {
        "signed-compare"
    }

    fn rewrites(&self, ctx: &mut Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        let Some(comparison) =
            Comparison::from_instr(&ctx.function.blocks[position.0].instructions[position.1])
        else {
            return vec![];
        };
        if !matches!(comparison.kind, Kind::Lt | Kind::Ge) {
            return vec![];
        }
        let x = comparison.left;
        let ty = ctx.function.locals[x].ty;
        let (unsigned, min, sign) = match ty {
            Ty::I8 => (Ty::U8, Value::I8(i8::MIN), 0x80),
            Ty::I16 => (Ty::U16, Value::I16(i16::MIN), 0x8000),
            _ => return vec![],
        };
        // Two constants are left for constant propagation to fold.
        if ctx.constant(&Operand::Local(x), ty).is_some() {
            return vec![];
        }
        let Some(constant) = ctx.constant(&comparison.right, ty) else {
            return vec![];
        };
        if constant == min {
            return vec![];
        }
        let Operand::Literal(IntegerLiteral { value: constant }) = to_literal(constant) else {
            unreachable!("to_literal makes a literal");
        };
        let literal = |value| Operand::Literal(IntegerLiteral::from(value));

        let u = ctx.fresh_local(unsigned);
        let mut instrs = vec![Instr::BitCast {
            target: u,
            source: x,
        }];
        let (kind, left, right) = if constant == 0 {
            // `x < 0` => `u >= SIGN`, `x >= 0` => `u < SIGN`
            let kind = match comparison.kind {
                Kind::Lt => Kind::Ge,
                _ => Kind::Lt,
            };
            (kind, u, literal(sign))
        } else {
            // `x < c` => `(u ^ SIGN) < (c ^ SIGN)`
            let biased = ctx.fresh_local(unsigned);
            instrs.push(Instr::BitXor {
                target: biased,
                left: u,
                right: literal(sign),
            });
            (comparison.kind, biased, literal(constant ^ sign))
        };
        instrs.push(
            Comparison {
                kind,
                target: comparison.target,
                left,
                right,
            }
            .into_instr(),
        );
        vec![Rewrite::replace_with(position, instrs)]
    }
}
