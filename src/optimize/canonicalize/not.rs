//! Puts bitwise complements into the `Not` opcode, e.g. `x ^ 255` becomes `!x`.
//!
//! With `MAX` being all bits set for the operand type (signed types included):
//! - **Not:** `x ^ MAX` becomes `!x`, as does `MAX ^ x`.
//! - **Cancellation:** `!!x` becomes a `Copy` of `x`.
//!
//! Only the outer instruction is rewritten. An inner `Not` is left for dead code elimination if
//! that was its only use. The target holds the same value as before, so none of this is
//! restricted by breakpoints.

use super::{Context, Rewrite, Rule};
use crate::consteval::Value;
use crate::mir::{BlockId, Instr, OpKind, Operand};

pub(super) struct Not;

impl Rule for Not {
    fn name(&self) -> &'static str {
        "not"
    }

    fn rewrites(&self, ctx: &Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        canonical(
            ctx,
            &ctx.function.blocks[position.0].instructions[position.1],
        )
        .map(|instr| Rewrite { position, instr })
        .into_iter()
        .collect()
    }
}

/// The canonical form of `instr`, if it is a complement not already in it.
fn canonical(ctx: &Context, instr: &Instr) -> Option<Instr> {
    // `!!x` => `x`
    if let Instr::Not { target, source } = instr {
        return match ctx.definition(&Operand::Local(*source)) {
            Some(Instr::Not { source: inner, .. }) => Some(Instr::Copy {
                target: *target,
                source: Operand::Local(*inner),
            }),
            _ => None,
        };
    }

    // `x ^ MAX` => `!x`, with the constant as either operand.
    let binop = instr
        .as_binop()
        .filter(|binop| binop.op == OpKind::BitXor)?;
    let ty = ctx.operand_ty(&binop);
    let max = !Value::zeros(ty, ctx.ty_registry);
    let left = Operand::Local(binop.left);
    [(&left, &binop.right), (&binop.right, &left)]
        .into_iter()
        .find_map(|(operand, constant)| {
            let Operand::Local(source) = operand else {
                return None;
            };
            (ctx.constant(constant, ty).as_ref() == Some(&max)).then_some(Instr::Not {
                target: binop.target,
                source: *source,
            })
        })
}
