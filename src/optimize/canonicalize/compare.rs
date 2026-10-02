//! Normalizes comparisons so a constant sits on the right, where codegen can compare against it
//! as an immediate, and so they read the carry flag where that is no worse than the zero flag,
//! e.g. `5 < x` becomes `x >= 6` and `x == 0` becomes `x < 1`.
//!
//! With `MIN` and `MAX` being the bounds of the operand type (the signed bounds for a signed
//! type) and `c` a constant:
//! - **Constant on the left:** `c < x` becomes `x >= c + 1` and `c >= x` becomes `x < c + 1`. At
//!   `c == MAX` the result is `0` or `1` instead. `c == x` and `c != x` swap their operands.
//! - **Always constant:** `x < MIN` becomes `0` and `x >= MIN` becomes `1`.
//! - **`u1` operand:** `b == 1`, `b != 0` and `b >= 1` become a `Copy` of `b`. `b == 0`, `b != 1`
//!   and `b < 1` become `!b`, which is only a flip of the condition `b` is held as.
//! - **Arithmetic** (`==` and `!=` only, since wrapping makes it unsound for the ordered
//!   comparisons): `x + c1 == c2` becomes `x == c2 - c1`, `x - c1 == c2` becomes `x == c2 + c1`,
//!   `x ^ c1 == c2` becomes `x == c1 ^ c2` and `a - b == 0` becomes `a == b`. For the commutative
//!   `Add` and `BitXor` the constant may be the left operand (`c1 + x`).
//! - **Prefer carry:** `x == 0` becomes `x < 1`, `x != 0` becomes `x >= 1`, `x == MAX` becomes
//!   `x >= MAX` and `x != MAX` becomes `x < MAX`. Branching on either flag costs the same, but
//!   keeping the result as a value is far cheaper from carry. This is limited to unsigned
//!   operands of at most 8 bits: a 16-bit ordered comparison is larger than a 16-bit test for
//!   zero, and a signed one has to bias both of its operands first.
//! - **Single-bit mask:** with `m` a constant of exactly one set bit, `(x & m) == m` becomes
//!   `(x & m) != 0` and `(x & m) != m` becomes `(x & m) == 0`, for every integer width. Codegen
//!   lowers a comparison of such a mask with `0` to `BIT`, which sets the zero flag from the
//!   tested bit, so this takes priority over preferring carry: no comparison of a single-bit
//!   mask is rewritten to read the carry flag.
//!
//! Only the comparison is rewritten. An inner instruction it no longer reads is left for dead code
//! elimination if that was its only use. The target holds the same value as before, so none of
//! this is restricted by breakpoints.

use super::{Context, Rewrite, Rule, to_literal};
use crate::ast::IntegerLiteral;
use crate::consteval::Value;
use crate::mir::{BlockId, Instr, LocalId, OpKind, Operand};
use crate::ty::Ty;

pub(super) struct Compare;

impl Rule for Compare {
    fn name(&self) -> &'static str {
        "compare"
    }

    fn rewrites(&self, ctx: &mut Context, position: (BlockId, usize)) -> Vec<Rewrite> {
        Comparison::from_instr(&ctx.function.blocks[position.0].instructions[position.1])
            .and_then(|comparison| canonical(ctx, &comparison))
            .map(|instr| Rewrite::replace(position, instr))
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Eq,
    Ne,
    Lt,
    Ge,
}

/// A comparison [`Instr`], destructured.
pub(super) struct Comparison {
    pub(super) kind: Kind,
    pub(super) target: LocalId,
    pub(super) left: LocalId,
    pub(super) right: Operand,
}

impl Comparison {
    pub(super) fn from_instr(instr: &Instr) -> Option<Self> {
        let (kind, target, left, right) = match instr {
            Instr::Eq {
                target,
                left,
                right,
            } => (Kind::Eq, target, left, right),
            Instr::Ne {
                target,
                left,
                right,
            } => (Kind::Ne, target, left, right),
            Instr::Lt {
                target,
                left,
                right,
            } => (Kind::Lt, target, left, right),
            Instr::Ge {
                target,
                left,
                right,
            } => (Kind::Ge, target, left, right),
            _ => return None,
        };
        Some(Comparison {
            kind,
            target: *target,
            left: *left,
            right: right.clone(),
        })
    }

    /// Rebuilds the instruction, the inverse of [`Comparison::from_instr`].
    pub(super) fn into_instr(self) -> Instr {
        let Comparison {
            kind,
            target,
            left,
            right,
        } = self;
        match kind {
            Kind::Eq => Instr::Eq {
                target,
                left,
                right,
            },
            Kind::Ne => Instr::Ne {
                target,
                left,
                right,
            },
            Kind::Lt => Instr::Lt {
                target,
                left,
                right,
            },
            Kind::Ge => Instr::Ge {
                target,
                left,
                right,
            },
        }
    }
}

/// The values of an integer type that the rewrites single out.
struct Bounds {
    min: Value,
    max: Value,
    one: Value,
}

impl Bounds {
    /// `None` if `ty` is not an integer type.
    fn of(ctx: &Context, ty: Ty) -> Option<Self> {
        let (min, max) = match ty {
            Ty::U1 | Ty::U2 | Ty::U4 | Ty::U8 | Ty::U16 => {
                let zero = Value::zeros(ty, ctx.ty_registry);
                (zero.clone(), !zero)
            }
            Ty::I8 => (Value::I8(i8::MIN), Value::I8(i8::MAX)),
            Ty::I16 => (Value::I16(i16::MIN), Value::I16(i16::MAX)),
            _ => return None,
        };
        let bytes = usize::from(ty.bit_size(ctx.ty_registry).padded_bytes());
        let one = Value::from_bytes(ty, ctx.ty_registry, &1u32.to_le_bytes()[..bytes]);
        Some(Bounds { min, max, one })
    }
}

/// The canonical form of `comparison`, if it is not already in it.
fn canonical(ctx: &Context, comparison: &Comparison) -> Option<Instr> {
    let ty = ctx.function.locals[comparison.left].ty;
    let bounds = Bounds::of(ctx, ty)?;
    let left = ctx.constant(&Operand::Local(comparison.left), ty);
    let right = ctx.constant(&comparison.right, ty);
    match (left, right) {
        (Some(constant), None) => constant_on_the_left(comparison, &bounds, constant),
        (None, Some(constant)) => constant_on_the_right(ctx, comparison, ty, &bounds, constant),
        // Two constants are left for constant propagation to fold.
        (Some(_), Some(_)) | (None, None) => None,
    }
}

/// Moves `constant`, the left operand of `comparison`, to the right.
fn constant_on_the_left(
    comparison: &Comparison,
    bounds: &Bounds,
    mut constant: Value,
) -> Option<Instr> {
    let Operand::Local(x) = comparison.right else {
        return None;
    };
    let kind = match comparison.kind {
        // `c == x` => `x == c`, `c != x` => `x != c`
        Kind::Eq | Kind::Ne => comparison.kind,
        // `MAX < x` => `0`, `MAX >= x` => `1`
        Kind::Lt | Kind::Ge if constant == bounds.max => {
            return Some(Instr::Copy {
                target: comparison.target,
                source: Operand::Literal(IntegerLiteral::from(u32::from(
                    comparison.kind == Kind::Ge,
                ))),
            });
        }
        // `c < x` => `x >= c + 1`
        Kind::Lt => {
            constant += bounds.one.clone();
            Kind::Ge
        }
        // `c >= x` => `x < c + 1`
        Kind::Ge => {
            constant += bounds.one.clone();
            Kind::Lt
        }
    };
    Some(
        Comparison {
            kind,
            target: comparison.target,
            left: x,
            right: to_literal(constant),
        }
        .into_instr(),
    )
}

/// Simplifies `comparison`, whose right operand is `constant` and whose left operand is not a
/// constant.
fn constant_on_the_right(
    ctx: &Context,
    comparison: &Comparison,
    ty: Ty,
    bounds: &Bounds,
    constant: Value,
) -> Option<Instr> {
    let x = comparison.left;
    let kind = comparison.kind;

    // `x < MIN` => `0`, `x >= MIN` => `1`
    if matches!(kind, Kind::Lt | Kind::Ge) && constant == bounds.min {
        return Some(Instr::Copy {
            target: comparison.target,
            source: Operand::Literal(IntegerLiteral::from(u32::from(kind == Kind::Ge))),
        });
    }

    if ty == Ty::U1 {
        // The ordered comparisons only get here with a constant of `1`.
        let holds_when_set = match kind {
            Kind::Eq => constant == bounds.max,
            Kind::Ne => constant == bounds.min,
            Kind::Ge => true,
            Kind::Lt => false,
        };
        return Some(if holds_when_set {
            Instr::Copy {
                target: comparison.target,
                source: Operand::Local(x),
            }
        } else {
            Instr::Not {
                target: comparison.target,
                source: x,
            }
        });
    }

    if !matches!(kind, Kind::Eq | Kind::Ne) {
        return None;
    }
    if let Some((left, right)) = through_arithmetic(ctx, x, ty, &constant) {
        return Some(
            Comparison {
                kind,
                target: comparison.target,
                left,
                right,
            }
            .into_instr(),
        );
    }

    // This is decided before preferring carry, which would otherwise take the test for zero away
    // from the zero flag.
    if let Some(mask) = single_bit_mask(ctx, x, ty) {
        if constant != mask {
            return None;
        }
        // `(x & m) == m` => `(x & m) != 0`, `(x & m) != m` => `(x & m) == 0`
        return Some(
            Comparison {
                kind: if kind == Kind::Eq { Kind::Ne } else { Kind::Eq },
                target: comparison.target,
                left: x,
                right: to_literal(Value::zeros(ty, ctx.ty_registry)),
            }
            .into_instr(),
        );
    }

    if ty.is_signed_int() || ty.bit_size(ctx.ty_registry) > 8 {
        return None;
    }
    let (kind, constant) = match kind {
        // `x == 0` => `x < 1`
        Kind::Eq if constant == bounds.min => (Kind::Lt, bounds.one.clone()),
        // `x != 0` => `x >= 1`
        Kind::Ne if constant == bounds.min => (Kind::Ge, bounds.one.clone()),
        // `x == MAX` => `x >= MAX`
        Kind::Eq if constant == bounds.max => (Kind::Ge, constant),
        // `x != MAX` => `x < MAX`
        Kind::Ne if constant == bounds.max => (Kind::Lt, constant),
        _ => return None,
    };
    Some(
        Comparison {
            kind,
            target: comparison.target,
            left: x,
            right: to_literal(constant),
        }
        .into_instr(),
    )
}

/// The operands of the equality equivalent to `x == constant` that reads the operands of the
/// instruction defining `x` in place of `x`, if that instruction can be undone on both sides.
fn through_arithmetic(
    ctx: &Context,
    x: LocalId,
    ty: Ty,
    constant: &Value,
) -> Option<(LocalId, Operand)> {
    let inner = ctx.definition(&Operand::Local(x))?.as_binop()?;
    let left = Operand::Local(inner.left);
    // `a - b == 0` => `a == b`
    if inner.op == OpKind::Sub && *constant == Value::zeros(ty, ctx.ty_registry) {
        return Some((inner.left, inner.right));
    }

    // `(other, c1)` operand pairs: the right operand is always eligible as the constant, the
    // left only when the opcode is commutative.
    let mut candidates = vec![(&left, &inner.right)];
    if inner.op.is_commutative() {
        candidates.push((&inner.right, &left));
    }
    candidates.into_iter().find_map(|(other, c1)| {
        let Operand::Local(other) = other else {
            return None;
        };
        let c1 = ctx.constant(c1, ty)?;
        let mut constant = constant.clone();
        match inner.op {
            // `x + c1 == c2` => `x == c2 - c1`
            OpKind::Add => constant -= c1,
            // `x - c1 == c2` => `x == c2 + c1`
            OpKind::Sub => constant += c1,
            // `x ^ c1 == c2` => `x == c1 ^ c2`
            OpKind::BitXor => constant ^= c1,
            OpKind::BitAnd | OpKind::BitOr => return None,
        }
        Some((*other, to_literal(constant)))
    })
}

/// The mask, if `x` is defined as an `&` with a constant mask of exactly one bit.
fn single_bit_mask(ctx: &Context, x: LocalId, ty: Ty) -> Option<Value> {
    let Some(Instr::BitAnd { right: mask, .. }) = ctx.definition(&Operand::Local(x)) else {
        return None;
    };
    ctx.constant(mask, ty).filter(|mask| {
        let bytes = mask.clone().into_vec();
        bytes.iter().map(|byte| byte.count_ones()).sum::<u32>() == 1
    })
}
