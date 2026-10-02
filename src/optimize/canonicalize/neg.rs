//! Puts arithmetic negations into the `Neg` opcode, e.g. `0 - x` becomes `-x`.
//!
//! - **Neg:** `0 - x` and `!x + 1` become `-x`, as does `1 + !x`. This is limited to the 8- and
//!   16-bit integer types, since narrower types have no `Neg` lowering.
//! - **Cancellation:** `--x` becomes a `Copy` of `x`.
//!
//! Only the outer instruction is rewritten. An inner `Not` or `Neg` is left for dead code
//! elimination if that was its only use. The target holds the same value as before, so none of
//! this is restricted by breakpoints.

use super::{Context, Rewrite, Rule};
use crate::consteval::Value;
use crate::mir::{BlockId, Instr, OpKind, Operand};
use crate::ty::Ty;

pub(super) struct Neg;

impl Rule for Neg {
    fn name(&self) -> &'static str {
        "neg"
    }

    fn rewrites(&self, ctx: &mut Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        canonical(
            ctx,
            &ctx.function.blocks[position.0].instructions[position.1],
        )
        .map(|instr| Rewrite::replace(position, instr))
        .into_iter()
        .collect()
    }
}

/// The canonical form of `instr`, if it is a negation not already in it.
fn canonical(ctx: &Context, instr: &Instr) -> Option<Instr> {
    // `--x` => `x`
    if let Instr::Neg { target, source } = instr {
        return match ctx.definition(&Operand::Local(*source)) {
            Some(Instr::Neg { source: inner, .. }) => Some(Instr::Copy {
                target: *target,
                source: Operand::Local(*inner),
            }),
            _ => None,
        };
    }

    let binop = instr.as_binop()?;
    let ty = ctx.operand_ty(&binop);
    if !matches!(ty, Ty::U8 | Ty::I8 | Ty::U16 | Ty::I16) {
        return None;
    }
    let target = binop.target;
    let zero = Value::zeros(ty, ctx.ty_registry);
    let left = Operand::Local(binop.left);
    match binop.op {
        // `0 - x` => `-x`
        OpKind::Sub => {
            let Operand::Local(source) = binop.right else {
                return None;
            };
            (ctx.constant(&left, ty) == Some(zero)).then_some(Instr::Neg { target, source })
        }
        // `!x + 1` => `-x`, with the `Not` as either operand.
        OpKind::Add => {
            let mut one = zero;
            one -= !Value::zeros(ty, ctx.ty_registry);
            [(&left, &binop.right), (&binop.right, &left)]
                .into_iter()
                .find_map(|(operand, constant)| {
                    let Some(Instr::Not { source, .. }) = ctx.definition(operand) else {
                        return None;
                    };
                    (ctx.constant(constant, ty).as_ref() == Some(&one)).then_some(Instr::Neg {
                        target,
                        source: *source,
                    })
                })
        }
        OpKind::BitAnd | OpKind::BitOr | OpKind::BitXor => None,
    }
}
