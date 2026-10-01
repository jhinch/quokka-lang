//! Simplifies instructions whose result is already known from one operand, e.g. `x + 0` becomes
//! a copy of `x` and `x & 0` becomes `0`.
//!
//! With `MAX` being all bits set for the operand type (signed types included):
//! - **Identity:** `x + 0`, `x - 0`, `x | 0`, `x ^ 0` and `x & MAX` become a `Copy` of `x`, as do
//!   `x & x` and `x | x`.
//! - **Absorbing:** `x & 0` becomes `0` and `x | MAX` becomes `MAX`.
//! - **Self-inverse:** `x ^ x` and `x - x` become `0`.
//! - **Self-comparison:** `x == x` and `x >= x` become `1`, `x != x` and `x < x` become `0`.
//!
//! For a commutative opcode the constant may instead be the left operand (`0 + x`).
//!
//! The target holds the same value as before, so none of this is restricted by breakpoints.

use super::{Context, Rewrite, Rule, to_literal};
use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{BinOp, BlockId, Instr, LocalId, OpKind, Operand};

pub(super) struct Identity;

impl Rule for Identity {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn rewrites(&self, ctx: &Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        let instr = &ctx.function.blocks[position.0].instructions[position.1];
        let simplified = match instr.as_binop() {
            Some(binop) => simplify_binop(ctx, &binop).map(|source| (binop.target, source)),
            None => simplify_self_comparison(instr),
        };
        let Some((target, source)) = simplified else {
            return vec![];
        };
        vec![Rewrite {
            position,
            instr: Instr::Copy { target, source },
        }]
    }
}

/// The operand `binop`'s target is always equal to, if there is one.
fn simplify_binop(ctx: &Context, binop: &BinOp) -> Option<Operand> {
    let ty = ctx.operand_ty(binop);
    let zero = || Value::zeros(ty, ctx.ty_registry);
    let max = || !Value::zeros(ty, ctx.ty_registry);
    let left = Operand::Local(binop.left);

    if matches!(binop.right, Operand::Local(right) if right == binop.left) {
        return match binop.op {
            OpKind::BitAnd | OpKind::BitOr => Some(left),
            OpKind::BitXor | OpKind::Sub => Some(to_literal(zero())),
            OpKind::Add => None,
        };
    }

    // `(constant, other)` operand pairs: the right operand is always eligible as the constant,
    // the left only when `op` is commutative.
    let mut candidates = vec![(&binop.right, &left)];
    if binop.op.is_commutative() {
        candidates.push((&left, &binop.right));
    }
    for (constant, other) in candidates {
        let Some(constant) = ctx.constant(constant, ty) else {
            continue;
        };
        match binop.op {
            OpKind::Add | OpKind::Sub | OpKind::BitOr | OpKind::BitXor if constant == zero() => {
                return Some(other.clone());
            }
            OpKind::BitAnd if constant == max() => return Some(other.clone()),
            OpKind::BitAnd if constant == zero() => return Some(to_literal(zero())),
            OpKind::BitOr if constant == max() => return Some(to_literal(max())),
            _ => {}
        }
    }
    None
}

/// The target and constant result of `instr`, if it compares a local with itself.
fn simplify_self_comparison(instr: &Instr) -> Option<(LocalId, Operand)> {
    let (target, left, right, result) = match instr {
        Instr::Eq {
            target,
            left,
            right,
        }
        | Instr::Ge {
            target,
            left,
            right,
        } => (target, left, right, 1),
        Instr::Ne {
            target,
            left,
            right,
        }
        | Instr::Lt {
            target,
            left,
            right,
        } => (target, left, right, 0),
        _ => return None,
    };
    matches!(right, Operand::Local(right) if right == left)
        .then(|| (*target, Operand::Literal(IntegerLiteral::from(result))))
}
