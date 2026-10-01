//! Picks one opcode for each addition or subtraction, so the other rules and codegen each only
//! have to recognize one shape, e.g. `x - 5` becomes `x + 251`.
//!
//! - **Literal subtraction:** `x - c` becomes `x + (-c)`, with the constant negated at compile
//!   time under wrapping arithmetic. `Add` is the canonical form for a constant term.
//! - **Combining:** `a + (-b)` becomes `a - b` and `a - (-b)` becomes `a + b`. For the
//!   commutative `Add` the operands may be the other way around (`(-b) + a`).
//!
//! Only the outer instruction is rewritten, to read the `Neg`'s source directly. The `Neg` is left
//! for dead code elimination if that was its only use. The target holds the same value as before,
//! so none of this is restricted by breakpoints.

use super::{Context, Rewrite, Rule, to_literal};
use crate::consteval::Value;
use crate::mir::{BinOp, BlockId, Instr, LocalId, OpKind, Operand};

pub(super) struct Additive;

impl Rule for Additive {
    fn name(&self) -> &'static str {
        "additive"
    }

    fn rewrites(&self, ctx: &Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        ctx.function.blocks[position.0].instructions[position.1]
            .as_binop()
            .and_then(|binop| canonical(ctx, &binop))
            .map(|instr| Rewrite { position, instr })
            .into_iter()
            .collect()
    }
}

/// The local `operand` is the negation of, if it is defined by a `Neg`.
fn negated(ctx: &Context, operand: &Operand) -> Option<LocalId> {
    match ctx.definition(operand) {
        Some(Instr::Neg { source, .. }) => Some(*source),
        _ => None,
    }
}

/// The canonical form of `binop`, if it is an `Add` or `Sub` not already in it.
fn canonical(ctx: &Context, binop: &BinOp) -> Option<Instr> {
    let target = binop.target;
    let left = Operand::Local(binop.left);
    match binop.op {
        OpKind::Sub => {
            let ty = ctx.operand_ty(binop);
            // `x - c` => `x + (-c)`
            if let Some(constant) = ctx.constant(&binop.right, ty) {
                let mut negated = Value::zeros(ty, ctx.ty_registry);
                negated -= constant;
                return Some(Instr::Add {
                    target,
                    left: binop.left,
                    right: to_literal(negated),
                });
            }
            // `a - (-b)` => `a + b`
            let b = negated(ctx, &binop.right)?;
            Some(Instr::Add {
                target,
                left: binop.left,
                right: Operand::Local(b),
            })
        }
        // `a + (-b)` => `a - b`, with the `Neg` as either operand.
        OpKind::Add => [(&binop.right, &left), (&left, &binop.right)]
            .into_iter()
            .find_map(|(operand, other)| {
                let b = negated(ctx, operand)?;
                let Operand::Local(a) = other else {
                    return None;
                };
                Some(Instr::Sub {
                    target,
                    left: *a,
                    right: Operand::Local(b),
                })
            }),
        OpKind::BitAnd | OpKind::BitOr | OpKind::BitXor => None,
    }
}
