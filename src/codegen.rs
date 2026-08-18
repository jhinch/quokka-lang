use crate::attr::{RamSectionKind, RomSectionKind};
use crate::lir::{
    BlockId, CARRY_FLAG_BIT, CARRY_FLAG_MASK, CarryFlagId, CodeSectionBuilder, DataSectionData,
    FlagId, Ld8Source, Ld16Source, Lir, LirBuilder, Operand8, R8Id, R16Id, RamRegionKind,
    RamSectionData, RomRegionKind, SIGN_MASK, SectionId, Terminator, ZERO_FLAG_BIT, ZERO_FLAG_MASK,
    ZeroFlagId,
};
use crate::mir;
use crate::multi::{MultiError, MultiResultBuilder};
use crate::num::u3;
use crate::ty::{Ty, TyRegistry};
use slotmap::{SecondaryMap, SparseSecondaryMap};

pub struct CodeGen {
    ty_registry: TyRegistry,
    result_builder: MultiResultBuilder,
    lir: LirBuilder,
}

impl CodeGen {
    pub fn generate(mut self, mir: &mir::Mir) -> Result<Lir, MultiError> {
        self.generate_from_mir(mir);
        self.result_builder.take().finish(()).into_result()?;
        Ok(self.lir.build())
    }

    fn generate_from_mir(&mut self, mir: &mir::Mir) {
        let mut global_to_section = SecondaryMap::new();
        for (global_id, global) in &mir.globals {
            global_to_section.insert(global_id, self.generate_from_global(global));
        }
        let code_section_id = self.lir.reserve_code_section();
        let mut code_section = CodeSectionBuilder::default();
        let mut functions = SecondaryMap::new();
        for (function_id, function) in &mir.functions {
            let block = code_section.new_function(function.name.clone());
            functions.insert(function_id, (None, block));
        }
        for (function_id, function) in &mir.functions {
            self.generate_from_function(
                function,
                functions[function_id].1,
                &global_to_section,
                &functions,
                &mut code_section,
            );
        }
        self.lir.seal_code_section(code_section_id, code_section);
    }

    fn generate_from_global(&mut self, global: &mir::Global) -> SectionId {
        match &global.attrs {
            mir::GlobalAttributes::Rom(attrs) => self.lir.insert_data_section(DataSectionData {
                region: match attrs.section {
                    RomSectionKind::Auto => None,
                    RomSectionKind::RomX => Some(RomRegionKind::RomX(None)),
                    RomSectionKind::Rom0 => Some(RomRegionKind::Rom0),
                },
                address: None,
                data: global.value.clone(),
            }),
            mir::GlobalAttributes::Ram(attrs) => self.lir.insert_ram_section(RamSectionData {
                region: match attrs.section {
                    RamSectionKind::Auto => RamRegionKind::default(),
                    RamSectionKind::WRam0 => RamRegionKind::WRam0,
                    RamSectionKind::VRam => RamRegionKind::VRam,
                    RamSectionKind::HRam => RamRegionKind::HRam,
                    RamSectionKind::WRamX => RamRegionKind::WRamX(None),
                    RamSectionKind::SRam => RamRegionKind::SRam,
                    RamSectionKind::Oam => RamRegionKind::Oam,
                },
                address: 0,
            }),
        }
    }

    fn generate_from_function(
        &mut self,
        function: &mir::Function,
        start_block: BlockId,
        _global_to_section: &SecondaryMap<mir::GlobalId, SectionId>,
        functions: &SecondaryMap<mir::FunctionId, (Option<SectionId>, BlockId)>,
        lir: &mut CodeSectionBuilder,
    ) {
        let mut blocks = SecondaryMap::new();
        let mut locals = Locals::default();
        blocks.insert(function.start_block, start_block);
        for mir_block in function.traverse_blocks() {
            blocks.insert(
                mir_block,
                if function.start_block == mir_block {
                    start_block
                } else {
                    lir.new_block()
                },
            );
        }
        for mir_block in function.traverse_blocks() {
            let block = blocks[mir_block];
            for instruction in &function.blocks[mir_block].instructions {
                self.generate_from_instr(
                    function,
                    instruction,
                    block,
                    lir,
                    &Ids {
                        functions,
                        blocks: &blocks,
                    },
                    &mut locals,
                );
            }
            self.generate_from_terminator(
                function,
                &function.blocks[mir_block].terminator,
                block,
                lir,
                &Ids {
                    functions,
                    blocks: &blocks,
                },
                &mut locals,
            );
        }
    }

    fn generate_from_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) {
        match instruction
            .target_local()
            .map(|target| function.locals[target].ty)
        {
            Some(Ty::U1) => {
                self.generate_from_u1_instr(function, instruction, block, lir, ids, locals);
            }
            Some(Ty::U2) => todo!("{instruction:?} {:?}", Ty::U2),
            Some(Ty::U4) => todo!("{instruction:?} {:?}", Ty::U4),
            Some(Ty::U8 | Ty::I8) => {
                self.generate_from_scalar8_instr(function, instruction, block, lir, ids, locals);
            }
            Some(Ty::U16 | Ty::I16) => {
                self.generate_from_scalar16_instr(function, instruction, block, lir, ids, locals);
            }
            Some(ty) if ty.is_struct() => todo!("{instruction:?} {:?}", ty),
            Some(ty) if ty.is_array() => todo!("{instruction:?} {:?}", ty),
            Some(ty) => unimplemented!("{instruction:?} {:?}", ty),
            None => todo!("{instruction:?}"),
        }
    }

    fn generate_from_terminator(
        &mut self,
        _function: &mir::Function,
        terminator: &mir::Terminator,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) {
        match terminator {
            mir::Terminator::Return => {
                lir.set_terminator(block, Terminator::Ret);
            }
            mir::Terminator::Call {
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
            } => {
                let (callee_section, callee_block) = ids.functions[*callee];
                let return_block = ids.blocks[*mir_return_block];
                lir.set_terminator(
                    block,
                    Terminator::Call {
                        callee_section,
                        callee_block,
                        return_block,
                    },
                );
            }
            mir::Terminator::Call {
                callee: mir::Callee::Local(callee),
                return_block: mir_return_block,
            } => {
                let callee = locals.r16s[*callee];
                let return_block = ids.blocks[*mir_return_block];
                lir.set_terminator(
                    block,
                    Terminator::CallR16 {
                        callee,
                        return_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                source: mir_source,
                condition: mir_condition,
                callee: mir::Callee::FnPtr(callee),
                return_block: mir_return_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.r1_to_flag(*mir_source, block, lir, locals);
                let (callee_section, callee_block) = ids.functions[*callee];
                let return_block = ids.blocks[*mir_return_block];
                let else_block = ids.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::CallIf {
                        source,
                        negate: *mir_condition != condition,
                        callee_section,
                        callee_block,
                        return_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::CallIf {
                callee: mir::Callee::Local(_),
                ..
            } => unimplemented!(
                "{terminator:?} as it should be rewritten within the optimization phase"
            ),
            mir::Terminator::Jump {
                target_block: mir_target_block,
            } => {
                let target_block = ids.blocks[*mir_target_block];
                lir.set_terminator(
                    block,
                    Terminator::Jp {
                        block: target_block,
                    },
                );
            }
            mir::Terminator::JumpIf {
                source: mir_source,
                condition: mir_condition,
                then_block: mir_then_block,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.r1_to_flag(*mir_source, block, lir, locals);
                let then_block = ids.blocks[*mir_then_block];
                let else_block = ids.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::JpIf {
                        source,
                        negate: *mir_condition != condition,
                        then_block,
                        else_block,
                    },
                );
            }
            mir::Terminator::ReturnIf {
                source: mir_source,
                condition: mir_condition,
                else_block: mir_else_block,
            } => {
                let (source, condition) = self.r1_to_flag(*mir_source, block, lir, locals);
                let else_block = ids.blocks[*mir_else_block];
                lir.set_terminator(
                    block,
                    Terminator::RetIf {
                        source,
                        negate: *mir_condition != condition,
                        else_block,
                    },
                );
            }
        }
    }

    fn generate_from_u1_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) {
        match instruction {
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = lir.insert_ld8(block, Ld8Source::Operand(Operand8::Literal(0)));
                locals.r1s.insert(
                    *target,
                    (
                        R1::Clean {
                            r8,
                            bit: u3::new(ZERO_FLAG_BIT),
                        },
                        mir::Condition::Zero,
                    ),
                );
            }
            mir::Instr::ReturnValue { .. } => todo!("{instruction:?}"),
            mir::Instr::Phi { .. } => todo!("{instruction:?}"),
            mir::Instr::Load { .. } => todo!("{instruction:?}"),
            mir::Instr::Store { .. } => unreachable!("{instruction:?}"),
            mir::Instr::Copy { target, source } => {
                let (source, bit, state, condition) =
                    self.r1_to_operand(source, block, lir, locals);
                let r8 = lir.insert_ld8(block, Ld8Source::Operand(source));
                locals
                    .r1s
                    .insert(*target, (R1::from((r8, bit, state)), condition));
            }
            mir::Instr::Read { .. } => todo!("{instruction:?}"),
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let (flag, condition) = locals.r1s[*source];
                locals.r1s.insert(*target, (flag, !condition));
            }
            mir::Instr::Neg { .. } => unimplemented!("neg unsupported on u1"),
            mir::Instr::RotateLeft { target, source }
            | mir::Instr::RotateRight { target, source } => {
                let (r8, bit, state, condition) = self.r1_to_r8(*source, block, lir, locals);
                locals
                    .r1s
                    .insert(*target, (R1::from((r8, bit, state)), condition));
            }
            mir::Instr::ShiftLeft { target, source: _ }
            | mir::Instr::ShiftRight { target, source: _ } => {
                let r8 =
                    lir.insert_ld8(block, Ld8Source::Operand(Operand8::Literal(ZERO_FLAG_MASK)));
                locals.r1s.insert(
                    *target,
                    (
                        R1::Clean {
                            r8,
                            bit: u3::new(ZERO_FLAG_BIT),
                        },
                        mir::Condition::NotZero,
                    ),
                );
            }
            mir::Instr::Eq {
                target,
                left,
                right,
            } => {
                let (z, condition) =
                    self.eq_from_comparison_op(function, *left, right, block, lir, ids, locals);
                locals.r1s.insert(*target, (R1::from(z), condition));
            }
            mir::Instr::Ne {
                target,
                left,
                right,
            } => {
                let (z, condition) =
                    self.eq_from_comparison_op(function, *left, right, block, lir, ids, locals);
                locals.r1s.insert(*target, (R1::from(z), !condition));
            }
            mir::Instr::Lt {
                target,
                left,
                right,
            } => {
                let (c, condition) =
                    self.lt_from_comparison_op(function, *left, right, block, lir, ids, locals);
                locals.r1s.insert(*target, (R1::from(c), condition));
            }
            mir::Instr::Le { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::Ge {
                target,
                left,
                right,
            } => {
                let (c, condition) =
                    self.lt_from_comparison_op(function, *left, right, block, lir, ids, locals);
                locals.r1s.insert(*target, (R1::from(c), !condition));
            }
            mir::Instr::Gt { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (left, left_mask, left_condition) =
                    self.r1_to_r8_clean(*left, block, lir, locals);
                let (right, right_mask, right_condition) =
                    self.r1_to_operand_clean(right, block, lir, locals);
                if left_condition == right_condition {
                    if left_condition == mir::Condition::NotZero {
                        let (_r8, z, _c) = lir.insert_and(block, left, right);
                        locals
                            .r1s
                            .insert(*target, (R1::from(z), mir::Condition::NotZero));
                    } else {
                        let (_r8, z, _c) = lir.insert_or(block, left, right);
                        locals
                            .r1s
                            .insert(*target, (R1::from(z), mir::Condition::Zero));
                    }
                } else {
                    todo!(
                        "{instruction:?}, left_mask={left_mask}, right_mask={right_mask}, left_condition={left_condition:?}, right_condition={right_condition:?}"
                    );
                }
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (left, left_mask, left_condition) =
                    self.r1_to_r8_clean(*left, block, lir, locals);
                let (right, right_mask, right_condition) =
                    self.r1_to_operand_clean(right, block, lir, locals);
                if left_condition == right_condition {
                    if left_condition == mir::Condition::NotZero {
                        let (_r8, z, _c) = lir.insert_or(block, left, right);
                        locals
                            .r1s
                            .insert(*target, (R1::from(z), mir::Condition::NotZero));
                    } else {
                        let (_r8, z, _c) = lir.insert_and(block, left, right);
                        locals
                            .r1s
                            .insert(*target, (R1::from(z), mir::Condition::Zero));
                    }
                } else {
                    todo!(
                        "{instruction:?}, left_mask={left_mask}, right_mask={right_mask}, left_condition={left_condition:?}, right_condition={right_condition:?}"
                    );
                }
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            }
            | mir::Instr::Add {
                target,
                left,
                right,
            }
            | mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (left, left_mask, left_condition) =
                    self.r1_to_r8_clean(*left, block, lir, locals);
                let (right, right_mask, right_condition) =
                    self.r1_to_operand_clean(right, block, lir, locals);
                if left_condition == right_condition {
                    let (_r8, z, _c) = lir.insert_xor(block, left, right);
                    locals
                        .r1s
                        .insert(*target, (R1::from(z), mir::Condition::NotZero));
                } else {
                    todo!(
                        "{instruction:?}, left_mask={left_mask}, right_mask={right_mask}, left_condition={left_condition:?}, right_condition={right_condition:?}"
                    );
                }
            }
        }
    }

    fn generate_from_scalar8_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        _ids: &Ids<'_>,
        locals: &mut Locals,
    ) {
        match instruction {
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r8 = lir.insert_ld8(block, Ld8Source::Operand(Operand8::Literal(0)));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::ReturnValue { .. } => todo!("{instruction:?}"),
            mir::Instr::Phi { .. } => todo!("{instruction:?}"),
            mir::Instr::Load { .. } => todo!("{instruction:?}"),
            mir::Instr::Store { .. } => unreachable!("{instruction:?}"),
            mir::Instr::Copy { target, source } => {
                let r8 = lir.insert_ld8(
                    block,
                    Ld8Source::Operand(self.r8_to_operand(source, locals)),
                );
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Read { .. } => todo!("{instruction:?}"),
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let r8 = lir.insert_cpl(block, locals.r8s[*source]);
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Neg { target, source } => {
                let (r8, _z, _c) =
                    lir.insert_xor(block, locals.r8s[*source], Operand8::Literal(SIGN_MASK));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::RotateLeft { target, source } => {
                let (r8, _z, _c) = lir.insert_rlc(block, locals.r8s[*source]);
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::RotateRight { target, source } => {
                let (r8, _z, _c) = lir.insert_rrc(block, locals.r8s[*source]);
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (r8, _z, _c) = lir.insert_sla(block, locals.r8s[*source]);
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::ShiftRight { target, source } => {
                let (r8, _z, _c) = if function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, locals.r8s[*source])
                } else {
                    lir.insert_srl(block, locals.r8s[*source])
                };
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Le { .. }
            | mir::Instr::Ge { .. }
            | mir::Instr::Gt { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_and(block, locals.r8s[*left], self.r8_to_operand(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_or(block, locals.r8s[*left], self.r8_to_operand(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_xor(block, locals.r8s[*left], self.r8_to_operand(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_add(block, locals.r8s[*left], self.r8_to_operand(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_sub(block, locals.r8s[*left], self.r8_to_operand(right, locals));
                locals.r8s.insert(*target, r8);
            }
        }
    }

    fn generate_from_scalar16_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) {
        match instruction {
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r16 = lir.insert_ld16(block, Ld16Source::Literal(0));
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::ReturnValue { .. } => todo!("{instruction:?}"),
            mir::Instr::Phi { .. } => todo!("{instruction:?}"),
            mir::Instr::Load { .. } => todo!("{instruction:?}"),
            mir::Instr::Store { .. } => unreachable!("{instruction:?}"),
            mir::Instr::Copy { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(source, block, lir, ids, locals);
                let target_high =
                    lir.insert_ld8(block, Ld8Source::Operand(Operand8::Direct(source_high)));
                let target_low =
                    lir.insert_ld8(block, Ld8Source::Operand(Operand8::Direct(source_low)));
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::Read { .. } => todo!("{instruction:?}"),
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let target_low = lir.insert_cpl(block, source_low);
                let target_high = lir.insert_cpl(block, source_high);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::Neg { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let (target_high, _z, _c) =
                    lir.insert_xor(block, source_high, Operand8::Literal(SIGN_MASK));
                let r16 = lir.insert_join(block, target_high, source_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::RotateLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let (_r8, _z, c) = lir.insert_sla(block, source_high);
                let (target_low, _z, c) = lir.insert_rl(block, source_low, c);
                let (target_high, _z, _c) = lir.insert_rl(block, source_high, c);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::RotateRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let (_r8, _z, c) = lir.insert_sra(block, source_low);
                let (target_high, _z, c) = lir.insert_rr(block, source_high, c);
                let (target_low, _z, _c) = lir.insert_rr(block, source_low, c);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::ShiftLeft { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let (target_low, _z, c) = lir.insert_sla(block, source_low);
                let (target_high, _z, _c) = lir.insert_rl(block, source_high, c);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::ShiftRight { target, source } => {
                let (source_high, source_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*source), block, lir, ids, locals);
                let (target_high, _z, c) = if function.locals[*target].ty.is_signed_int() {
                    lir.insert_sra(block, source_high)
                } else {
                    lir.insert_srl(block, source_high)
                };
                let (target_low, _z, _c) = lir.insert_rr(block, source_low, c);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::Eq { .. }
            | mir::Instr::Ne { .. }
            | mir::Instr::Lt { .. }
            | mir::Instr::Le { .. }
            | mir::Instr::Ge { .. }
            | mir::Instr::Gt { .. } => {
                unimplemented!("{instruction:?} comparisons always produce U1")
            }
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (target_low, _z, _c) =
                    lir.insert_and(block, left_low, Operand8::Direct(right_low));
                let (target_high, _z, _c) =
                    lir.insert_and(block, left_high, Operand8::Direct(right_high));
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (target_low, _z, _c) =
                    lir.insert_or(block, left_low, Operand8::Direct(right_low));
                let (target_high, _z, _c) =
                    lir.insert_or(block, left_high, Operand8::Direct(right_high));
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (target_low, _z, _c) =
                    lir.insert_xor(block, left_low, Operand8::Direct(right_low));
                let (target_high, _z, _c) =
                    lir.insert_xor(block, left_high, Operand8::Direct(right_high));
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let left = locals.r16s[*left];
                let right = self.r16_from_operand(right, block, lir, ids, locals);
                let (r16, _c) = lir.insert_add16(block, left, right);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(*left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (target_high, _z, c) =
                    lir.insert_sub(block, left_high, Operand8::Direct(right_high));
                let (target_low, _z, _c) =
                    lir.insert_sbc(block, left_low, Operand8::Direct(right_low), c);
                let r16 = lir.insert_join(block, target_high, target_low);
                locals.r16s.insert(*target, r16);
            }
        }
    }

    fn r8_to_operand(&self, right: &mir::Operand, locals: &Locals) -> Operand8 {
        match right {
            mir::Operand::Local(right) => Operand8::Direct(locals.r8s[*right]),
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => Operand8::Literal(right.value as u8),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 8-bit instruction using fn ptr")
            }
        }
    }

    fn r16_from_operand(
        &self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &Locals,
    ) -> R16Id {
        match right {
            mir::Operand::Local(right) => locals.r16s[*right],
            mir::Operand::Literal(right) =>
            {
                #[allow(clippy::cast_possible_truncation)]
                lir.insert_ld16(block, Ld16Source::Literal(right.value as u16))
            }
            mir::Operand::FnPtr(function) => {
                let (section, block) = ids.functions[*function];
                lir.insert_ld16(block, Ld16Source::FnPtr { section, block })
            }
        }
    }

    fn r16_to_high_low(
        &self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &Locals,
    ) -> (R8Id, R8Id) {
        let right = self.r16_from_operand(right, block, lir, ids, locals);
        lir.insert_split(block, right)
    }

    fn r1_to_flag(
        &self,
        local: mir::LocalId,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (FlagId, mir::Condition) {
        let (flag, condition) = locals.r1s[local];
        match flag {
            R1::Flag(flag) => (flag, condition),
            R1::Dirty { r8, bit } | R1::Clean { r8, bit } => {
                let z = lir.insert_bit(block, bit, r8);
                (FlagId::from(z), !condition)
            }
        }
    }

    fn r1_to_r8(
        &self,
        right: mir::LocalId,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (R8Id, u3, R1State, mir::Condition) {
        let (right, condition) = locals.r1s[right];
        match right {
            R1::Flag(FlagId::Zero(z_in)) => {
                let right = lir.insert_ld_zero(block, z_in);
                (right, u3::new(ZERO_FLAG_BIT), R1State::Dirty, condition)
            }
            R1::Flag(FlagId::Carry(c_in)) => {
                let right = lir.insert_ld_carry(block, c_in);
                (right, u3::new(CARRY_FLAG_BIT), R1State::Dirty, condition)
            }
            R1::Dirty { r8: right, bit } => (right, bit, R1State::Dirty, condition),
            R1::Clean { r8: right, bit } => (right, bit, R1State::Clean, condition),
        }
    }

    fn r1_to_r8_clean(
        &self,
        right: mir::LocalId,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (R8Id, u3, mir::Condition) {
        let (right, condition) = locals.r1s[right];
        match right {
            R1::Flag(FlagId::Zero(z_in)) => {
                let right = lir.insert_ld_zero(block, z_in);
                let (right, _z, _c) =
                    lir.insert_and(block, right, Operand8::Literal(ZERO_FLAG_MASK));
                (right, u3::new(ZERO_FLAG_BIT), condition)
            }
            R1::Flag(FlagId::Carry(c_in)) => {
                let right = lir.insert_ld_carry(block, c_in);
                let (right, _z, _c) =
                    lir.insert_and(block, right, Operand8::Literal(CARRY_FLAG_MASK));
                (right, u3::new(CARRY_FLAG_BIT), condition)
            }
            R1::Dirty { r8: right, bit } => {
                let (right, _z, _c) =
                    lir.insert_and(block, right, Operand8::Literal(1 << u8::from(bit)));
                (right, bit, condition)
            }
            R1::Clean { r8: right, bit } => (right, bit, condition),
        }
    }

    fn r1_to_operand(
        &self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (Operand8, u3, R1State, mir::Condition) {
        match right {
            mir::Operand::Local(right) => {
                let (right, bit, state, condition) = self.r1_to_r8(*right, block, lir, locals);
                (Operand8::Direct(right), bit, state, condition)
            }
            mir::Operand::Literal(right) => (
                Operand8::Literal(if right.value != 0 { ZERO_FLAG_MASK } else { 0 }),
                u3::new(ZERO_FLAG_BIT),
                R1State::Clean,
                mir::Condition::NotZero,
            ),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 1-bit instruction using fn ptr")
            }
        }
    }

    fn r1_to_operand_clean(
        &self,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &Locals,
    ) -> (Operand8, u3, mir::Condition) {
        match right {
            mir::Operand::Local(right) => {
                let (right, bit, condition) = self.r1_to_r8_clean(*right, block, lir, locals);
                (Operand8::Direct(right), bit, condition)
            }
            mir::Operand::Literal(right) => (
                Operand8::Literal(if right.value != 0 { ZERO_FLAG_MASK } else { 0 }),
                u3::new(ZERO_FLAG_BIT),
                mir::Condition::NotZero,
            ),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform 1-bit instruction using fn ptr")
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn eq_from_comparison_op(
        &mut self,
        function: &mir::Function,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) -> (ZeroFlagId, mir::Condition) {
        match function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (z, _c) = self.cp_from_r8(function, left, right, block, lir, locals);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                let (z, _c) = self.cp_from_r8(function, left, right, block, lir, locals);
                (z, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 16 => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (target_low, _z, _c) =
                    lir.insert_xor(block, left_low, Operand8::Direct(right_low));
                let (target_high, _z, _c) =
                    lir.insert_xor(block, left_high, Operand8::Direct(right_high));
                let (_r8, z, _c) = lir.insert_or(block, target_high, Operand8::Direct(target_low));
                (z, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lt_from_comparison_op(
        &mut self,
        function: &mir::Function,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        ids: &Ids<'_>,
        locals: &mut Locals,
    ) -> (CarryFlagId, mir::Condition) {
        match function.locals[left].ty {
            Ty::U1 | Ty::U2 | Ty::U4 => {
                let (_z, c) = self.cp_from_r8(function, left, right, block, lir, locals);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                let (_z, c) = self.cp_from_r8(function, left, right, block, lir, locals);
                (c, mir::Condition::NotZero)
            }
            ty if ty.bit_size(&self.ty_registry) == 16 => {
                let (left_high, left_low) =
                    self.r16_to_high_low(&mir::Operand::Local(left), block, lir, ids, locals);
                let (right_high, right_low) = self.r16_to_high_low(right, block, lir, ids, locals);
                let (_r16, _z, c) = lir.insert_sub(block, left_high, Operand8::Direct(right_high));
                let (_r16, _z, c) = lir.insert_sbc(block, left_low, Operand8::Direct(right_low), c);
                (c, mir::Condition::NotZero)
            }
            ty => unimplemented!("{ty:?}"),
        }
    }

    fn cp_from_r8(
        &mut self,
        function: &mir::Function,
        left: mir::LocalId,
        right: &mir::Operand,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &mut Locals,
    ) -> (ZeroFlagId, CarryFlagId) {
        match function.locals[left].ty {
            Ty::U1 => {
                let (mut left, left_bit, left_condition) =
                    self.r1_to_r8_clean(left, block, lir, locals);
                let (mut right, right_bit, right_condition) =
                    self.r1_to_operand_clean(right, block, lir, locals);
                if left_condition != right_condition {
                    if left_condition == mir::Condition::Zero {
                        let (r8, _z, _c) =
                            lir.insert_xor(block, left, Operand8::Literal(1 << u8::from(left_bit)));
                        left = r8;
                    } else {
                        right = match right {
                            Operand8::Ptr(_) => {
                                unreachable!("r1_to_operand should not produce Operand8::Ptr")
                            }
                            Operand8::Direct(right) => {
                                let (right, _z, _c) = lir.insert_xor(
                                    block,
                                    right,
                                    Operand8::Literal(1 << u8::from(right_bit)),
                                );
                                Operand8::Direct(right)
                            }
                            Operand8::Literal(value) => {
                                Operand8::Literal(value ^ (1 << u8::from(right_bit)))
                            }
                        };
                    }
                }
                lir.insert_cp(block, left, right)
            }
            Ty::U2 => todo!("{left:?} {right:?} u2"),
            Ty::U4 => todo!("{left:?} {right:?} u4"),
            ty if ty.bit_size(&self.ty_registry) == 8 => {
                lir.insert_cp(block, locals.r8s[left], self.r8_to_operand(right, locals))
            }
            ty => unimplemented!("{ty:?}"),
        }
    }
}

struct Ids<'a> {
    functions: &'a SecondaryMap<mir::FunctionId, (Option<SectionId>, BlockId)>,
    blocks: &'a SecondaryMap<mir::BlockId, BlockId>,
}

#[derive(Default)]
struct Locals {
    r1s: SparseSecondaryMap<mir::LocalId, (R1, mir::Condition)>,
    r8s: SparseSecondaryMap<mir::LocalId, R8Id>,
    r16s: SparseSecondaryMap<mir::LocalId, R16Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum R1 {
    Flag(FlagId),
    /// flag contained within `r8` at `bit` with all other bits being undefined.
    Dirty {
        r8: R8Id,
        bit: u3,
    },
    /// flag contained within `r8` at `bit` with all other bits guaranteed to be `0`.
    Clean {
        r8: R8Id,
        bit: u3,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum R1State {
    Dirty,
    Clean,
}

impl From<FlagId> for R1 {
    fn from(value: FlagId) -> Self {
        R1::Flag(value)
    }
}

impl From<ZeroFlagId> for R1 {
    fn from(value: ZeroFlagId) -> Self {
        R1::Flag(value.into())
    }
}

impl From<CarryFlagId> for R1 {
    fn from(value: CarryFlagId) -> Self {
        R1::Flag(value.into())
    }
}

impl From<(R8Id, u3, R1State)> for R1 {
    fn from((r8, bit, state): (R8Id, u3, R1State)) -> Self {
        match state {
            R1State::Dirty => R1::Dirty { r8, bit },
            R1State::Clean => R1::Clean { r8, bit },
        }
    }
}
