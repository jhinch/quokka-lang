use crate::mir;
use crate::mir::{Condition, GlobalId, GlobalPlaceProjection, Instr, Operand, Terminator};
use crate::ty::{Ty, TyRegistry};
use slotmap::{SecondaryMap, SparseSecondaryMap};

#[must_use]
pub fn evaluate(
    function: &mir::Function,
    globals: &SparseSecondaryMap<GlobalId, (Ty, &[u8])>,
    ty_registry: &TyRegistry,
) -> Vec<u8> {
    let mut local_slots = SecondaryMap::new();
    let return_local = function
        .return_local
        .expect("cannot evaluate unit function");
    for local in function.locals.keys() {
        local_slots.insert(
            local,
            vec![
                0u8;
                function.locals[local]
                    .ty
                    .bit_size(ty_registry)
                    .next_multiple_of(8) as usize
                    / 8
            ],
        );
    }
    let mut previous_block = None;
    let mut block = function.start_block;
    loop {
        for instruction in function.blocks[block].instructions.iter() {
            match instruction {
                Instr::Undefined { .. } | Instr::ReturnValue { .. } | Instr::Store { .. } => {
                    unimplemented!("Unsupported instruction {instruction:?}")
                }
                Instr::Phi { target, sources } => {
                    let source = sources
                        [previous_block.expect("phi cannot be resolved when no previous block")];
                    let [target, source] = local_slots
                        .get_disjoint_mut([*target, source])
                        .expect("slots");
                    target.as_mut_slice().copy_from_slice(source);
                }
                Instr::Load {
                    target,
                    source: place,
                } => {
                    let (mut ty, mut source) = globals[place.global];
                    for projection in &place.projections {
                        match projection {
                            GlobalPlaceProjection::Field(field) => {
                                let field = &ty_registry.struct_ty_data(ty).fields[field];
                                let offset = field.bit_offset as usize / 8;
                                source = &source[offset
                                    ..(offset + (field.ty.bit_size(ty_registry) as usize / 8))];
                                ty = field.ty;
                            }
                            GlobalPlaceProjection::Index(_) => todo!("array index"),
                        }
                    }
                    let target = &mut local_slots[*target];
                    target.copy_from_slice(source);
                }
                Instr::Copy { target, source } => match source {
                    Operand::Local(source) => {
                        let [target, source] = local_slots
                            .get_disjoint_mut([*target, *source])
                            .expect("slots");
                        target.as_mut_slice().copy_from_slice(source);
                    }
                    Operand::Literal(int) => {
                        let source = int.value.to_le_bytes();
                        let target = &mut local_slots[*target];
                        let byte_len = target.len().min(source.len());
                        target[..byte_len].copy_from_slice(&source[..byte_len]);
                    }
                    Operand::FnPtr(_) => unimplemented!("Unsupported instruction {instruction:?}"),
                },
                Instr::Not { .. } => todo!("not"),
                Instr::Neg { .. } => todo!("neg"),
                Instr::RotateLeft { .. } => todo!("rotateleft"),
                Instr::RotateRight { .. } => todo!("rotateright"),
                Instr::ShiftLeft { .. } => todo!("shiftleft"),
                Instr::ShiftRight { .. } => todo!("shiftright"),
                Instr::Eq { .. } => todo!("eq"),
                Instr::Ne { .. } => todo!("ne"),
                Instr::Lt { .. } => todo!("lt"),
                Instr::Le { .. } => todo!("le"),
                Instr::Ge { .. } => todo!("ge"),
                Instr::Gt { .. } => todo!("gt"),
                Instr::BitAnd { .. } => todo!("bitand"),
                Instr::BitOr { .. } => todo!("bitor"),
                Instr::BitXor { .. } => todo!("bitxor"),
                Instr::Add { .. } => todo!("add"),
                Instr::Sub { .. } => todo!("sub"),
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
                let mask = local_slots[*source].iter().fold(0, |a, b| a | b);
                let cond = match condition {
                    Condition::Zero => mask == 0,
                    Condition::NotZero => mask != 0,
                };
                if cond { *then_block } else { *else_block }
            }
            Terminator::ReturnIf {
                source,
                condition,
                else_block,
            } => {
                let mask = local_slots[*source].iter().fold(0, |a, b| a | b);
                let cond = match condition {
                    Condition::Zero => mask == 0,
                    Condition::NotZero => mask != 0,
                };
                if cond { break } else { *else_block }
            }
        };
        previous_block = Some(block);
        block = next_block;
    }
    local_slots
        .remove(return_local)
        .expect("return slot not found")
}
