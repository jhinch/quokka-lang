use crate::mir;
use crate::mir::{Condition, GlobalId, Instr, LocalId, Operand, PlaceProjection, Terminator};
use crate::num::{BitCount, u2, u4};
use crate::ty::{Ty, TyRegistry};
use slotmap::{SecondaryMap, SparseSecondaryMap};
use std::cmp::Ordering;
use std::num::NonZeroU16;
use std::ops::{AddAssign, BitAndAssign, BitOrAssign, BitXorAssign, Neg, Not, SubAssign};

#[must_use]
pub fn evaluate(
    function: &mir::Function,
    globals: &SparseSecondaryMap<GlobalId, (Ty, &[u8])>,
    ty_registry: &TyRegistry,
) -> Vec<u8> {
    let mut local_slots = SecondaryMap::<LocalId, Option<Value>>::new();
    let return_local = function
        .return_local
        .expect("cannot evaluate unit function");
    for local in function.locals.keys() {
        local_slots.insert(local, None);
    }
    let mut previous_block = None;
    let mut block = function.start_block;
    loop {
        for instruction in function.blocks[block].instructions.iter() {
            match instruction {
                Instr::Nop => {}
                Instr::Undefined { .. }
                | Instr::ReturnValue { .. }
                | Instr::Store { .. }
                | Instr::Transfer { .. }
                | Instr::Fill { .. } => {
                    unimplemented!("Unsupported instruction {instruction:?}")
                }
                Instr::Alloc { target } => {
                    let source = Value::zeros(function.locals[*target].ty, ty_registry);
                    local_slots[*target].set(source);
                }
                Instr::Phi { target, sources } => {
                    let source = sources
                        [previous_block.expect("phi cannot be resolved when no previous block")];
                    let source = local_slots[source].check_init(source, function).clone();
                    local_slots[*target].set(source);
                }
                Instr::Load {
                    target,
                    source,
                    projections,
                } => {
                    let (global_ty, source) = globals[*source];
                    let source = Value::from_bytes(global_ty, ty_registry, source);
                    let source =
                        read_projections(source, projections, &local_slots, function, ty_registry);
                    local_slots[*target].set(source);
                }
                Instr::Read {
                    target,
                    source,
                    projections,
                } => {
                    let source = local_slots[*source].check_init(*source, function).clone();
                    let source =
                        read_projections(source, projections, &local_slots, function, ty_registry);
                    local_slots[*target].set(source);
                }
                Instr::Write {
                    target,
                    left,
                    projections,
                    right,
                } => {
                    let left_value = local_slots[*left].check_init(*left, function).clone();
                    let right_ty =
                        projected_ty(function.locals[*left].ty, projections, ty_registry);
                    let right =
                        Value::from_operand(&local_slots, function, right_ty, ty_registry, right);
                    let new_bytes = write_projections(
                        left_value,
                        projections,
                        &right.into_vec(),
                        &local_slots,
                        function,
                        ty_registry,
                    );
                    let source =
                        Value::from_bytes(function.locals[*target].ty, ty_registry, &new_bytes);
                    local_slots[*target].set(source);
                }
                Instr::Copy { target, source } => {
                    let source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        source,
                    );
                    local_slots[*target].set(source);
                }
                Instr::Not { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = !source;
                    local_slots[*target].set(source);
                }
                Instr::Neg { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = -source;
                    local_slots[*target].set(source);
                }
                Instr::RotateLeft { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = source.rotate_left_one();
                    local_slots[*target].set(source);
                }
                Instr::RotateRight { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = source.rotate_right_one();
                    local_slots[*target].set(source);
                }
                Instr::ShiftLeft { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = source.shift_left_one();
                    local_slots[*target].set(source);
                }
                Instr::ShiftRight { target, source } => {
                    let mut source = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        &Operand::Local(*source),
                    );
                    source = source.shift_right_one();
                    local_slots[*target].set(source);
                }
                Instr::Eq {
                    target,
                    left,
                    right,
                } => {
                    let operand_ty = function.locals[*left].ty;
                    let left = local_slots[*left].check_init(*left, function);
                    let right =
                        Value::from_operand(&local_slots, function, operand_ty, ty_registry, right);
                    let source = Value::U1(left == &right);
                    local_slots[*target].set(source);
                }
                Instr::Ne {
                    target,
                    left,
                    right,
                } => {
                    let operand_ty = function.locals[*left].ty;
                    let left = local_slots[*left].check_init(*left, function);
                    let right =
                        Value::from_operand(&local_slots, function, operand_ty, ty_registry, right);
                    let source = Value::U1(left != &right);
                    local_slots[*target].set(source);
                }
                Instr::Lt {
                    target,
                    left,
                    right,
                } => {
                    let operand_ty = function.locals[*left].ty;
                    let left = local_slots[*left].check_init(*left, function);
                    let right =
                        Value::from_operand(&local_slots, function, operand_ty, ty_registry, right);
                    let source = Value::U1(left < &right);
                    local_slots[*target].set(source);
                }
                Instr::Ge {
                    target,
                    left,
                    right,
                } => {
                    let operand_ty = function.locals[*left].ty;
                    let left = local_slots[*left].check_init(*left, function);
                    let right =
                        Value::from_operand(&local_slots, function, operand_ty, ty_registry, right);
                    let source = Value::U1(left >= &right);
                    local_slots[*target].set(source);
                }
                Instr::BitAnd {
                    target,
                    left,
                    right,
                } => {
                    let left = local_slots[*left].check_init(*left, function);
                    let right = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        right,
                    );
                    let mut source = left.clone();
                    source &= right;
                    local_slots[*target].set(source);
                }
                Instr::BitOr {
                    target,
                    left,
                    right,
                } => {
                    let left = local_slots[*left].check_init(*left, function);
                    let right = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        right,
                    );
                    let mut source = left.clone();
                    source |= right;
                    local_slots[*target].set(source);
                }
                Instr::BitXor {
                    target,
                    left,
                    right,
                } => {
                    let left = local_slots[*left].check_init(*left, function);
                    let right = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        right,
                    );
                    let mut source = left.clone();
                    source ^= right;
                    local_slots[*target].set(source);
                }
                Instr::Add {
                    target,
                    left,
                    right,
                } => {
                    let left = local_slots[*left].check_init(*left, function);
                    let right = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        right,
                    );
                    let mut source = left.clone();
                    source += right;
                    local_slots[*target].set(source);
                }
                Instr::Sub {
                    target,
                    left,
                    right,
                } => {
                    let left = local_slots[*left].check_init(*left, function);
                    let right = Value::from_operand(
                        &local_slots,
                        function,
                        function.locals[*target].ty,
                        ty_registry,
                        right,
                    );
                    let mut source = left.clone();
                    source -= right;
                    local_slots[*target].set(source);
                }
            }
        }
        let terminator = &function.blocks[block].terminator;
        let next_block = match terminator {
            Terminator::Return => break,
            Terminator::Call { .. } | Terminator::CallIf { .. } => {
                unimplemented!("unsupported terminator {terminator:?}")
            }
            Terminator::Jump { target_block } => *target_block,
            Terminator::JumpIf {
                source,
                condition,
                then_block,
                else_block,
            } => {
                if local_slots[*source]
                    .check_init(*source, function)
                    .as_bool(*condition)
                {
                    *then_block
                } else {
                    *else_block
                }
            }
            Terminator::ReturnIf {
                source,
                condition,
                else_block,
            } => {
                if local_slots[*source]
                    .check_init(*source, function)
                    .as_bool(*condition)
                {
                    break;
                } else {
                    *else_block
                }
            }
        };
        previous_block = Some(block);
        block = next_block;
    }
    local_slots
        .remove(return_local)
        .flatten()
        .expect("return slot not set")
        .into_vec()
}

/// Narrows `value` down to the value at `projections`, resolving each field/index lookup in turn.
///
/// This is not particularly efficient as it clones for each projection. However, it should work
/// correctly for now.
fn read_projections(
    mut value: Value,
    projections: &[PlaceProjection],
    local_slots: &SecondaryMap<LocalId, Option<Value>>,
    function: &mir::Function,
    ty_registry: &TyRegistry,
) -> Value {
    for projection in projections {
        value = match projection {
            PlaceProjection::Field(field) => match value {
                Value::Struct { ty, bytes, .. } => {
                    let field = &ty_registry.struct_ty_data(ty).fields[field];
                    let offset = usize::from(field.bit_offset.bytes());
                    let width = usize::from(field.ty.bit_size(ty_registry).padded_bytes());
                    Value::from_bytes(field.ty, ty_registry, &bytes[offset..(offset + width)])
                }
                value => unimplemented!("cannot lookup field {field} on value {value:?}"),
            },
            PlaceProjection::Index(index) => match value {
                Value::Array { item_ty, bytes, .. } => {
                    let index =
                        Value::from_operand(local_slots, function, Ty::U16, ty_registry, index);
                    let index = match index {
                        Value::U16(index) => index as usize,
                        index => unimplemented!("cannot array index on with index {index:?}"),
                    };
                    let item_byte_width = usize::from(item_ty.bit_size(ty_registry).padded_bytes());
                    let offset = index * item_byte_width;
                    Value::from_bytes(
                        item_ty,
                        ty_registry,
                        &bytes[offset..(offset + item_byte_width)],
                    )
                }
                value => unimplemented!("cannot array index on value {value:?}"),
            },
        }
    }
    value
}

/// Returns the byte representation of `value` with `right_bytes` spliced in at `projections`.
///
/// This is not particularly efficient as it clones for each projection. However, it should work
/// correctly for now.
fn write_projections(
    value: Value,
    projections: &[PlaceProjection],
    right_bytes: &[u8],
    local_slots: &SecondaryMap<LocalId, Option<Value>>,
    function: &mir::Function,
    ty_registry: &TyRegistry,
) -> Vec<u8> {
    let Some((projection, rest)) = projections.split_first() else {
        return right_bytes.to_vec();
    };
    match projection {
        PlaceProjection::Field(field) => match value {
            Value::Struct { ty, mut bytes, .. } => {
                let field = &ty_registry.struct_ty_data(ty).fields[field];
                let offset = usize::from(field.bit_offset.bytes());
                let width = usize::from(field.ty.bit_size(ty_registry).padded_bytes());
                let child =
                    Value::from_bytes(field.ty, ty_registry, &bytes[offset..(offset + width)]);
                let child_bytes =
                    write_projections(child, rest, right_bytes, local_slots, function, ty_registry);
                bytes[offset..(offset + width)].copy_from_slice(&child_bytes);
                bytes
            }
            value => unimplemented!("cannot lookup field {field} on value {value:?}"),
        },
        PlaceProjection::Index(index) => match value {
            Value::Array {
                item_ty, mut bytes, ..
            } => {
                let index = Value::from_operand(local_slots, function, Ty::U16, ty_registry, index);
                let index = match index {
                    Value::U16(index) => index as usize,
                    index => unimplemented!("cannot array index on with index {index:?}"),
                };
                let item_byte_width = usize::from(item_ty.bit_size(ty_registry).padded_bytes());
                let offset = index * item_byte_width;
                let child = Value::from_bytes(
                    item_ty,
                    ty_registry,
                    &bytes[offset..(offset + item_byte_width)],
                );
                let child_bytes =
                    write_projections(child, rest, right_bytes, local_slots, function, ty_registry);
                bytes[offset..(offset + item_byte_width)].copy_from_slice(&child_bytes);
                bytes
            }
            value => unimplemented!("cannot array index on value {value:?}"),
        },
    }
}

/// Returns the type reached by resolving `projections` in turn, starting from `ty`.
fn projected_ty(mut ty: Ty, projections: &[PlaceProjection], ty_registry: &TyRegistry) -> Ty {
    for projection in projections {
        ty = match projection {
            PlaceProjection::Field(field) => ty_registry.struct_ty_data(ty).fields[field].ty,
            PlaceProjection::Index(_) => ty_registry.array_ty_data(ty).item_ty,
        };
    }
    ty
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    U1(bool),
    U2(u2),
    U4(u4),
    U8(u8),
    U16(u16),
    I8(i8),
    I16(i16),
    Array {
        item_ty: Ty,
        count: NonZeroU16,
        bytes: Vec<u8>,
    },
    Struct {
        ty: Ty,
        bit_width: BitCount,
        bytes: Vec<u8>,
    },
}

impl Value {
    #[must_use]
    pub fn zeros(ty: Ty, ty_registry: &TyRegistry) -> Self {
        match ty {
            Ty::U1 => Value::U1(false),
            Ty::U2 => Value::U2(u2::new(0)),
            Ty::U4 => Value::U4(u4::new(0)),
            Ty::U8 => Value::U8(0),
            Ty::U16 => Value::U16(0),
            Ty::I8 => Value::I8(0),
            Ty::I16 => Value::I16(0),
            ty if ty.is_array() => {
                let array_ty_data = ty_registry.array_ty_data(ty);
                Value::Array {
                    item_ty: array_ty_data.item_ty,
                    count: array_ty_data.count,
                    bytes: vec![0; usize::from(ty.bit_size(ty_registry).padded_bytes())],
                }
            }
            ty if ty.is_struct() => {
                let bit_width = ty.bit_size(ty_registry);
                Value::Struct {
                    ty,
                    bit_width,
                    bytes: vec![0; usize::from(bit_width.padded_bytes())],
                }
            }
            ty => unimplemented!("unsupported type {}", ty.display(ty_registry)),
        }
    }

    #[must_use]
    pub fn from_operand(
        slots: &SecondaryMap<LocalId, Option<Value>>,
        function: &mir::Function,
        ty: Ty,
        ty_registry: &TyRegistry,
        operand: &Operand,
    ) -> Self {
        match operand {
            Operand::Local(local) => slots[*local].check_init(*local, function).clone(),
            Operand::Literal(int) => {
                let byte_width = usize::from(ty.bit_size(ty_registry).padded_bytes());
                Value::from_bytes(ty, ty_registry, &int.value.to_le_bytes()[..byte_width])
            }
            operand => unimplemented!("unsupported operand {operand:?}"),
        }
    }

    #[must_use]
    pub fn from_bytes(ty: Ty, ty_registry: &TyRegistry, bytes: &[u8]) -> Self {
        let bit_width = ty.bit_size(ty_registry);
        let byte_width = bit_width.padded_bytes();
        assert_eq!(
            bytes.len(),
            byte_width as usize,
            "{} has bit_width {bit_width} byte_width {byte_width} but got {} bytes",
            ty.display(ty_registry),
            bytes.len()
        );
        match ty {
            Ty::U1 => Value::U1(bytes[0] != 0),
            Ty::U2 => Value::U2(u2::try_from(bytes[0]).unwrap()),
            Ty::U4 => Value::U4(u4::try_from(bytes[0]).unwrap()),
            Ty::U8 => Value::U8(bytes[0]),
            Ty::U16 => Value::U16(u16::from_le_bytes(*bytes.as_array::<2>().unwrap())),
            Ty::I8 => Value::I8(i8::from_le_bytes(*bytes.as_array::<1>().unwrap())),
            Ty::I16 => Value::I16(i16::from_le_bytes(*bytes.as_array::<2>().unwrap())),
            ty if ty.is_array() => {
                let array_ty_data = ty_registry.array_ty_data(ty);
                Value::Array {
                    item_ty: array_ty_data.item_ty,
                    count: array_ty_data.count,
                    bytes: bytes.to_vec(),
                }
            }
            ty if ty.is_struct() => Value::Struct {
                ty,
                bit_width,
                bytes: bytes.to_vec(),
            },
            ty => unimplemented!("unsupported type {}", ty.display(ty_registry)),
        }
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Value::U1(value) => (value as u8).to_le_bytes().to_vec(),
            Value::U2(value) => value.to_le_bytes().to_vec(),
            Value::U4(value) => value.to_le_bytes().to_vec(),
            Value::U8(value) => value.to_le_bytes().to_vec(),
            Value::U16(value) => value.to_le_bytes().to_vec(),
            Value::I8(value) => value.to_le_bytes().to_vec(),
            Value::I16(value) => value.to_le_bytes().to_vec(),
            Value::Array { bytes, .. } | Value::Struct { bytes, .. } => bytes,
        }
    }

    #[must_use]
    pub fn as_bool(&self, condition: Condition) -> bool {
        match self {
            Value::U1(value) => match condition {
                Condition::Zero => !value,
                Condition::NotZero => *value,
            },
            value => unimplemented!("{value:?} is not a u1"),
        }
    }

    #[must_use]
    pub fn rotate_left_one(self) -> Self {
        match self {
            Value::U1(value) => Value::U1(value),
            Value::U2(value) => Value::U2(value << 1 | value >> 1),
            Value::U4(value) => Value::U4(value << 1 | value >> 3),
            Value::U8(value) => Value::U8(value.rotate_left(1)),
            Value::U16(value) => Value::U16(value.rotate_left(1)),
            Value::I8(value) => Value::I8(value.rotate_left(1)),
            Value::I16(value) => Value::I16(value.rotate_left(1)),
            value => unimplemented!("rotate_left operation not supported on {value:?}"),
        }
    }

    #[must_use]
    pub fn rotate_right_one(self) -> Self {
        match self {
            Value::U1(value) => Value::U1(value),
            Value::U2(value) => Value::U2(value >> 1 | value << 1),
            Value::U4(value) => Value::U4(value >> 1 | value << 3),
            Value::U8(value) => Value::U8(value.rotate_right(1)),
            Value::U16(value) => Value::U16(value.rotate_right(1)),
            Value::I8(value) => Value::I8(value.rotate_right(1)),
            Value::I16(value) => Value::I16(value.rotate_right(1)),
            value => unimplemented!("rotate_right operation not supported on {value:?}"),
        }
    }

    #[must_use]
    pub fn shift_left_one(self) -> Self {
        match self {
            Value::U1(_) => Value::U1(false),
            Value::U2(value) => Value::U2(value << 1),
            Value::U4(value) => Value::U4(value << 1),
            Value::U8(value) => Value::U8(value << 1),
            Value::U16(value) => Value::U16(value << 1),
            Value::I8(value) => Value::I8(value << 1),
            Value::I16(value) => Value::I16(value << 1),
            value => unimplemented!("shift_left operation not supported on {value:?}"),
        }
    }

    #[must_use]
    pub fn shift_right_one(self) -> Self {
        match self {
            Value::U1(_) => Value::U1(false),
            Value::U2(value) => Value::U2(value >> 1),
            Value::U4(value) => Value::U4(value >> 1),
            Value::U8(value) => Value::U8(value >> 1),
            Value::U16(value) => Value::U16(value >> 1),
            Value::I8(value) => Value::I8(value >> 1),
            Value::I16(value) => Value::I16(value >> 1),
            value => unimplemented!("shift_left operation not supported on {value:?}"),
        }
    }
}

impl Not for Value {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Value::U1(value) => Value::U1(!value),
            Value::U2(value) => Value::U2(!value),
            Value::U4(value) => Value::U4(!value),
            Value::U8(value) => Value::U8(!value),
            Value::U16(value) => Value::U16(!value),
            Value::I8(value) => Value::I8(!value),
            Value::I16(value) => Value::I16(!value),
            value => unimplemented!("not operation not supported on {value:?}"),
        }
    }
}

impl Neg for Value {
    type Output = Self;

    fn neg(self) -> Self::Output {
        match self {
            Value::I8(value) => Value::I8(-value),
            Value::I16(value) => Value::I16(-value),
            value => unimplemented!("neg operation not supported on {value:?}"),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Value::U1(this), Value::U1(other)) => this.partial_cmp(other),
            (Value::U2(this), Value::U2(other)) => this.partial_cmp(other),
            (Value::U4(this), Value::U4(other)) => this.partial_cmp(other),
            (Value::U8(this), Value::U8(other)) => this.partial_cmp(other),
            (Value::U16(this), Value::U16(other)) => this.partial_cmp(other),
            (Value::I8(this), Value::I8(other)) => this.partial_cmp(other),
            (Value::I16(this), Value::I16(other)) => this.partial_cmp(other),
            (this, other) if this == other => Some(Ordering::Equal),
            _ => None,
        }
    }
}

impl BitAndAssign for Value {
    fn bitand_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            (Value::U1(this), Value::U1(rhs)) => *this &= rhs,
            (Value::U2(this), Value::U2(rhs)) => *this &= rhs,
            (Value::U4(this), Value::U4(rhs)) => *this &= rhs,
            (Value::U8(this), Value::U8(rhs)) => *this &= rhs,
            (Value::U16(this), Value::U16(rhs)) => *this &= rhs,
            (Value::I8(this), Value::I8(rhs)) => *this &= rhs,
            (Value::I16(this), Value::I16(rhs)) => *this &= rhs,
            params => unimplemented!("bitand operation not supported on {params:?}"),
        }
    }
}

impl BitOrAssign for Value {
    fn bitor_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            (Value::U1(this), Value::U1(rhs)) => *this |= rhs,
            (Value::U2(this), Value::U2(rhs)) => *this |= rhs,
            (Value::U4(this), Value::U4(rhs)) => *this |= rhs,
            (Value::U8(this), Value::U8(rhs)) => *this |= rhs,
            (Value::U16(this), Value::U16(rhs)) => *this |= rhs,
            (Value::I8(this), Value::I8(rhs)) => *this |= rhs,
            (Value::I16(this), Value::I16(rhs)) => *this |= rhs,
            params => unimplemented!("bitor operation not supported on {params:?}"),
        }
    }
}

impl BitXorAssign for Value {
    fn bitxor_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            (Value::U1(this), Value::U1(rhs)) => *this ^= rhs,
            (Value::U2(this), Value::U2(rhs)) => *this ^= rhs,
            (Value::U4(this), Value::U4(rhs)) => *this ^= rhs,
            (Value::U8(this), Value::U8(rhs)) => *this ^= rhs,
            (Value::U16(this), Value::U16(rhs)) => *this ^= rhs,
            (Value::I8(this), Value::I8(rhs)) => *this ^= rhs,
            (Value::I16(this), Value::I16(rhs)) => *this ^= rhs,
            params => unimplemented!("bitxor operation not supported on {params:?}"),
        }
    }
}

impl AddAssign for Value {
    fn add_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            #[allow(clippy::suspicious_op_assign_impl)]
            (Value::U1(this), Value::U1(rhs)) => *this ^= rhs,
            (Value::U2(this), Value::U2(rhs)) => *this = this.wrapping_add(rhs),
            (Value::U4(this), Value::U4(rhs)) => *this = this.wrapping_add(rhs),
            (Value::U8(this), Value::U8(rhs)) => *this = this.wrapping_add(rhs),
            (Value::U16(this), Value::U16(rhs)) => *this = this.wrapping_add(rhs),
            (Value::I8(this), Value::I8(rhs)) => *this = this.wrapping_add(rhs),
            (Value::I16(this), Value::I16(rhs)) => *this = this.wrapping_add(rhs),
            params => unimplemented!("add operation not supported on {params:?}"),
        }
    }
}

impl SubAssign for Value {
    fn sub_assign(&mut self, rhs: Self) {
        match (self, rhs) {
            #[allow(clippy::suspicious_op_assign_impl)]
            (Value::U1(this), Value::U1(rhs)) => *this ^= rhs,
            (Value::U2(this), Value::U2(rhs)) => *this = this.wrapping_sub(rhs),
            (Value::U4(this), Value::U4(rhs)) => *this = this.wrapping_sub(rhs),
            (Value::U8(this), Value::U8(rhs)) => *this = this.wrapping_sub(rhs),
            (Value::U16(this), Value::U16(rhs)) => *this = this.wrapping_sub(rhs),
            (Value::I8(this), Value::I8(rhs)) => *this = this.wrapping_sub(rhs),
            (Value::I16(this), Value::I16(rhs)) => *this = this.wrapping_sub(rhs),
            params => unimplemented!("sub operation not supported on {params:?}"),
        }
    }
}

trait Slot {
    /// return the current value. Panics if uninitialized
    fn check_init(&self, id: LocalId, function: &mir::Function) -> &Value;
    /// sets the slot to the specified value
    fn set(&mut self, value: Value);
}

impl Slot for Option<Value> {
    fn check_init(&self, id: LocalId, function: &mir::Function) -> &Value {
        self.as_ref()
            .unwrap_or_else(|| panic!("{} not initialized", id.display(function)))
    }

    fn set(&mut self, value: Value) {
        *self = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Identifier, IntegerLiteral};
    use crate::mir::FunctionBuilder;

    fn new_builder(return_ty: Ty) -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from("f"), return_ty)
    }

    fn eval(function: &mir::Function) -> Vec<u8> {
        evaluate(function, &SparseSecondaryMap::new(), &TyRegistry::default())
    }

    #[test]
    fn evaluates_straight_line_arithmetic() {
        let mut fb = new_builder(Ty::U8);
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let a = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: a,
                source: Operand::Literal(IntegerLiteral::from(3)),
            },
        );
        fb.push_instr(
            block,
            Instr::Add {
                target: ret,
                left: a,
                right: Operand::Literal(IntegerLiteral::from(4)),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let function = fb.build();

        assert_eq!(eval(&function), vec![7]);
    }

    #[test]
    fn evaluates_jumpif_branch_selection() {
        let mut fb = new_builder(Ty::U8);
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);
        let cond = fb.new_local(Ty::U1);
        fb.push_instr(
            b1,
            Instr::Copy {
                target: cond,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(
            b1,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: b2,
                else_block: b3,
            },
        );
        fb.push_instr(
            b2,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(b2, Terminator::Return);
        fb.push_instr(
            b3,
            Instr::Copy {
                target: ret,
                source: Operand::Literal(IntegerLiteral::from(2)),
            },
        );
        fb.set_terminator(b3, Terminator::Return);
        fb.seal_block(b1);
        fb.seal_block(b2);
        fb.seal_block(b3);
        let function = fb.build();

        assert_eq!(eval(&function), vec![1]);
    }

    fn eval_comparison(
        left_value: u32,
        make_instr: impl FnOnce(LocalId, LocalId, Operand) -> Instr,
        right_literal: u32,
    ) -> Vec<u8> {
        let mut fb = new_builder(Ty::U1);
        let block = fb.new_block();
        let ret = fb.create_return_local(Ty::U1);
        let left = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: left,
                source: Operand::Literal(IntegerLiteral::from(left_value)),
            },
        );
        fb.push_instr(
            block,
            make_instr(
                ret,
                left,
                Operand::Literal(IntegerLiteral::from(right_literal)),
            ),
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let function = fb.build();
        eval(&function)
    }

    #[test]
    fn eq_against_a_literal_right_operand() {
        // 0u8 == 0 must be true ([1]).
        let result = eval_comparison(
            0,
            |target, left, right| Instr::Eq {
                target,
                left,
                right,
            },
            0,
        );
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn ne_against_a_literal_right_operand() {
        // 5u8 != 5 must be false ([0]).
        let result = eval_comparison(
            5,
            |target, left, right| Instr::Ne {
                target,
                left,
                right,
            },
            5,
        );
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn lt_against_a_literal_right_operand() {
        // 3u8 < 5 must be true ([1]).
        let result = eval_comparison(
            3,
            |target, left, right| Instr::Lt {
                target,
                left,
                right,
            },
            5,
        );
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn ge_against_a_literal_right_operand() {
        // 5u8 >= 5 must be true ([1]).
        let result = eval_comparison(
            5,
            |target, left, right| Instr::Ge {
                target,
                left,
                right,
            },
            5,
        );
        assert_eq!(result, vec![1]);
    }

    // --- loop evaluation, as a `const` declaration's initializer would need ---
    //
    // `loop` inside a `const` declaration requires the `const-loop` preview feature (see
    // `tests/cases/item/const_decl.toml` for coverage through real source). These tests build
    // the MIR shape a real `const X: u8 = { let i = 0; let sum = 0; loop { if i == LIMIT { break; }
    // sum = sum + i; i = i + 1; } yield sum; };` compiles to (phi-headed loop, `JumpIf` exit test,
    // back edge) directly, and evaluate it exactly like a real constant initializer would: no
    // globals at all, since a `const` can only depend on other already-resolved constants.
    //
    // sum(0..limit) via a loop, mirroring the sum-accumulator pattern used throughout this
    // codebase's own loop examples (e.g. `tests/cases/golden/collatz.q`).
    fn build_const_sum_loop(limit: u32) -> mir::Function {
        let mut fb = new_builder(Ty::U8);
        let entry = fb.new_block();
        let header = fb.new_block();
        let body = fb.new_block();
        let exit_block = fb.new_block();
        let ret = fb.create_return_local(Ty::U8);

        let i0 = fb.new_local(Ty::U8);
        let sum0 = fb.new_local(Ty::U8);
        fb.push_instr(
            entry,
            Instr::Copy {
                target: i0,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        fb.push_instr(
            entry,
            Instr::Copy {
                target: sum0,
                source: Operand::Literal(IntegerLiteral::from(0)),
            },
        );
        fb.set_terminator(
            entry,
            Terminator::Jump {
                target_block: header,
            },
        );

        // allocated now, defined later in `body` - LocalId creation doesn't require the
        // defining instruction to exist yet, only the phi's back-edge source entry.
        let i_next = fb.new_local(Ty::U8);
        let sum_next = fb.new_local(Ty::U8);

        let i_phi = fb.new_local(Ty::U8);
        let mut i_sources = SparseSecondaryMap::new();
        i_sources.insert(entry, i0);
        i_sources.insert(body, i_next);
        fb.push_instr(
            header,
            Instr::Phi {
                target: i_phi,
                sources: i_sources,
            },
        );

        let sum_phi = fb.new_local(Ty::U8);
        let mut sum_sources = SparseSecondaryMap::new();
        sum_sources.insert(entry, sum0);
        sum_sources.insert(body, sum_next);
        fb.push_instr(
            header,
            Instr::Phi {
                target: sum_phi,
                sources: sum_sources,
            },
        );

        let cond = fb.new_local(Ty::U1);
        fb.push_instr(
            header,
            Instr::Eq {
                target: cond,
                left: i_phi,
                right: Operand::Literal(IntegerLiteral::from(limit)),
            },
        );
        fb.set_terminator(
            header,
            Terminator::JumpIf {
                source: cond,
                condition: Condition::NotZero,
                then_block: exit_block,
                else_block: body,
            },
        );

        fb.push_instr(
            body,
            Instr::Add {
                target: sum_next,
                left: sum_phi,
                right: Operand::Local(i_phi),
            },
        );
        fb.push_instr(
            body,
            Instr::Add {
                target: i_next,
                left: i_phi,
                right: Operand::Literal(IntegerLiteral::from(1)),
            },
        );
        fb.set_terminator(
            body,
            Terminator::Jump {
                target_block: header,
            },
        );

        fb.push_instr(
            exit_block,
            Instr::Copy {
                target: ret,
                source: Operand::Local(sum_phi),
            },
        );
        fb.set_terminator(exit_block, Terminator::Return);

        fb.seal_block(entry);
        fb.seal_block(body);
        fb.seal_block(header);
        fb.seal_block(exit_block);
        fb.build()
    }

    fn triangular_number(limit: u32) -> u8 {
        (0..limit).fold(0u8, |acc, i| acc.wrapping_add(u8::try_from(i).unwrap()))
    }

    #[test]
    fn const_loop_with_zero_iterations_evaluates_correctly() {
        let function = build_const_sum_loop(0);
        assert_eq!(eval(&function), vec![triangular_number(0)]);
    }

    #[test]
    fn const_loop_with_one_iteration_evaluates_correctly() {
        let function = build_const_sum_loop(1);
        assert_eq!(eval(&function), vec![triangular_number(1)]);
    }

    #[test]
    fn const_loop_with_several_iterations_evaluates_correctly() {
        let function = build_const_sum_loop(5);
        assert_eq!(eval(&function), vec![triangular_number(5)]);
    }
}
