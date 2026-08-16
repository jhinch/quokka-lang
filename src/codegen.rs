use crate::attr::{RamSectionKind, RomSectionKind};
use crate::lir::{
    BlockId, CodeSectionBuilder, DataSectionData, FlagId, Ld8Source, Lir, LirBuilder, Operand8,
    R8Id, R16Id, RamRegionKind, RamSectionData, RomRegionKind, SectionId, Terminator,
};
use crate::mir;
use crate::multi::{MultiError, MultiResultBuilder};
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
        let mut function_to_block = SecondaryMap::new();
        for (function_id, function) in &mir.functions {
            let block = code_section.new_function(function.name.clone());
            function_to_block.insert(function_id, block);
        }
        for (function_id, function) in &mir.functions {
            self.generate_from_function(
                function,
                function_to_block[function_id],
                &global_to_section,
                &function_to_block,
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
        function_to_block: &SecondaryMap<mir::FunctionId, BlockId>,
        lir: &mut CodeSectionBuilder,
    ) {
        let mut mir_block_to_block = SecondaryMap::new();
        let mut locals = Locals::default();
        mir_block_to_block.insert(function.start_block, start_block);
        for mir_block in function.traverse_blocks() {
            mir_block_to_block.insert(
                mir_block,
                if function.start_block == mir_block {
                    start_block
                } else {
                    lir.new_block()
                },
            );
        }
        for mir_block in function.traverse_blocks() {
            let block = mir_block_to_block[mir_block];
            for instruction in &function.blocks[mir_block].instructions {
                self.generate_from_instr(function, instruction, block, lir, &mut locals);
            }
            let terminator = &function.blocks[mir_block].terminator;
            match terminator {
                mir::Terminator::Return => {
                    lir.set_terminator(block, Terminator::Ret);
                }
                mir::Terminator::Call {
                    callee: mir::Callee::FnPtr(callee),
                    return_block: mir_return_block,
                } => {
                    let callee_block = function_to_block[*callee];
                    let return_block = mir_block_to_block[*mir_return_block];
                    lir.set_terminator(
                        block,
                        Terminator::Call {
                            callee_section: None,
                            callee_block,
                            return_block,
                        },
                    );
                }
                mir::Terminator::Call { .. } => todo!("{terminator:?}"),
                mir::Terminator::CallIf {
                    source: mir_source,
                    condition: mir_condition,
                    callee: mir::Callee::FnPtr(callee),
                    return_block: mir_return_block,
                    else_block: mir_else_block,
                } => {
                    let (source, condition) = locals.flags[*mir_source];
                    let callee_block = function_to_block[*callee];
                    let return_block = mir_block_to_block[*mir_return_block];
                    let else_block = mir_block_to_block[*mir_else_block];
                    lir.set_terminator(
                        block,
                        Terminator::CallIf {
                            source,
                            negate: *mir_condition != condition,
                            callee_section: None,
                            callee_block,
                            return_block,
                            else_block,
                        },
                    );
                }
                mir::Terminator::CallIf { .. } => todo!("{terminator:?}"),
                mir::Terminator::Jump {
                    target_block: mir_target_block,
                } => {
                    let target_block = mir_block_to_block[*mir_target_block];
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
                    let (source, condition) = locals.flags[*mir_source];
                    let then_block = mir_block_to_block[*mir_then_block];
                    let else_block = mir_block_to_block[*mir_else_block];
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
                    let (source, condition) = locals.flags[*mir_source];
                    let else_block = mir_block_to_block[*mir_else_block];
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
    }

    fn generate_from_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &mut Locals,
    ) {
        match instruction
            .target_local()
            .map(|target| function.locals[target].ty)
        {
            Some(Ty::U1) => todo!("{instruction:?} {:?}", Ty::U1),
            Some(Ty::U2) => todo!("{instruction:?} {:?}", Ty::U2),
            Some(Ty::U4) => todo!("{instruction:?} {:?}", Ty::U4),
            Some(Ty::U8 | Ty::I8) => {
                self.generate_from_scalar8_instr(function, instruction, block, lir, locals);
            }
            Some(Ty::U16 | Ty::I16) => {
                self.generate_from_scalar16_instr(function, instruction, block, lir, locals);
            }
            Some(ty) if ty.is_struct() => todo!("{instruction:?} {:?}", ty),
            Some(ty) if ty.is_array() => todo!("{instruction:?} {:?}", ty),
            Some(ty) => unimplemented!("{instruction:?} {:?}", ty),
            None => todo!("{instruction:?}"),
        }
    }

    fn generate_from_scalar8_instr(
        &mut self,
        function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
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
                let r8 =
                    lir.insert_ld8(block, Ld8Source::Operand(self.to_operand8(source, locals)));
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
                    lir.insert_xor(block, locals.r8s[*source], Operand8::Literal(0b1000_0000));
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
                    lir.insert_srl(block, locals.r8s[*source])
                } else {
                    lir.insert_sra(block, locals.r8s[*source])
                };
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Eq {
                target,
                left,
                right,
            } => {
                let (z, _c) =
                    lir.insert_cp(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals
                    .flags
                    .insert(*target, (FlagId::from(z), mir::Condition::Zero));
            }
            mir::Instr::Ne {
                target,
                left,
                right,
            } => {
                let (z, _c) =
                    lir.insert_cp(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals
                    .flags
                    .insert(*target, (FlagId::from(z), mir::Condition::NotZero));
            }
            mir::Instr::Lt {
                target,
                left,
                right,
            } => {
                let (_z, c) =
                    lir.insert_cp(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals
                    .flags
                    .insert(*target, (FlagId::from(c), mir::Condition::NotZero));
            }
            mir::Instr::Le { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::Ge {
                target,
                left,
                right,
            } => {
                let (_z, c) =
                    lir.insert_cp(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals
                    .flags
                    .insert(*target, (FlagId::from(c), mir::Condition::Zero));
            }
            mir::Instr::Gt { .. } => unimplemented!(
                "{instruction:?} as it should be rewritten within the optimization phase"
            ),
            mir::Instr::BitAnd {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_and(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::BitOr {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_or(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::BitXor {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_xor(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Add {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_add(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals.r8s.insert(*target, r8);
            }
            mir::Instr::Sub {
                target,
                left,
                right,
            } => {
                let (r8, _z, _c) =
                    lir.insert_sub(block, locals.r8s[*left], self.to_operand8(right, locals));
                locals.r8s.insert(*target, r8);
            }
        }
    }

    fn generate_from_scalar16_instr(
        &mut self,
        _function: &mir::Function,
        instruction: &mir::Instr,
        block: BlockId,
        lir: &mut CodeSectionBuilder,
        locals: &mut Locals,
    ) {
        match instruction {
            mir::Instr::Undefined { .. } => unimplemented!("{instruction:?}"),
            mir::Instr::Alloc { target } => {
                let r16 = lir.insert_ld16(block, 0);
                locals.r16s.insert(*target, r16);
            }
            mir::Instr::ReturnValue { .. } => todo!("{instruction:?}"),
            mir::Instr::Phi { .. } => todo!("{instruction:?}"),
            mir::Instr::Load { .. } => todo!("{instruction:?}"),
            mir::Instr::Store { .. } => unreachable!("{instruction:?}"),
            mir::Instr::Copy { .. } => todo!("{instruction:?}"),
            mir::Instr::Read { .. } => todo!("{instruction:?}"),
            mir::Instr::Write { .. } => todo!("{instruction:?}"),
            mir::Instr::Not { .. } => todo!("{instruction:?}"),
            mir::Instr::Neg { .. } => todo!("{instruction:?}"),
            mir::Instr::RotateLeft { .. } => todo!("{instruction:?}"),
            mir::Instr::RotateRight { .. } => todo!("{instruction:?}"),
            mir::Instr::ShiftLeft { .. } => todo!("{instruction:?}"),
            mir::Instr::ShiftRight { .. } => todo!("{instruction:?}"),
            mir::Instr::Eq { .. } => todo!("{instruction:?}"),
            mir::Instr::Ne { .. } => todo!("{instruction:?}"),
            mir::Instr::Lt { .. } => todo!("{instruction:?}"),
            mir::Instr::Le { .. } => todo!("{instruction:?}"),
            mir::Instr::Ge { .. } => todo!("{instruction:?}"),
            mir::Instr::Gt { .. } => todo!("{instruction:?}"),
            mir::Instr::BitAnd { .. } => todo!("{instruction:?}"),
            mir::Instr::BitOr { .. } => todo!("{instruction:?}"),
            mir::Instr::BitXor { .. } => todo!("{instruction:?}"),
            mir::Instr::Add { .. } => todo!("{instruction:?}"),
            mir::Instr::Sub { .. } => todo!("{instruction:?}"),
        }
    }

    fn to_operand8(&self, right: &mir::Operand, locals: &Locals) -> Operand8 {
        match right {
            mir::Operand::Local(right) => Operand8::Direct(locals.r8s[*right]),
            #[allow(clippy::cast_possible_truncation)]
            mir::Operand::Literal(right) => Operand8::Literal(right.value as u8),
            mir::Operand::FnPtr(_) => {
                unimplemented!("cannot perform bit8 instructing using fn ptr")
            }
        }
    }
}

#[derive(Default)]
struct Locals {
    r8s: SparseSecondaryMap<mir::LocalId, R8Id>,
    r16s: SparseSecondaryMap<mir::LocalId, R16Id>,
    flags: SparseSecondaryMap<mir::LocalId, (FlagId, mir::Condition)>,
}
