use super::InstrCtx;
use crate::codegen::LoweredGlobal;
use crate::lir::{BlockId, HLId, Ld16Source, R8Id, SectionAddr};
use crate::mir;
use crate::num::{BitCount, u3};
use crate::ty::Ty;

impl InstrCtx<'_> {
    pub(super) fn projected_ty(
        &self,
        target: mir::GlobalId,
        projections: &[mir::PlaceProjection],
    ) -> Ty {
        let mut ty = self.mir.globals[target].ty;
        for projection in projections {
            ty = match projection {
                mir::PlaceProjection::Field(field) => {
                    self.ty_registry.struct_ty_data(ty).fields[field].ty
                }
                mir::PlaceProjection::Index(_) => self.ty_registry.array_ty_data(ty).item_ty,
            }
        }
        ty
    }

    pub(super) fn projected_global_to_addr(
        &mut self,
        global: mir::GlobalId,
        projections: &[mir::PlaceProjection],
        block: BlockId,
    ) -> (HLId, u3) {
        let mut bit_offset = BitCount::new(0);
        let mut r16 = None;
        let mut ty = self.mir.globals[global].ty;
        for project in projections {
            match project {
                mir::PlaceProjection::Field(field) => {
                    let field_data = &self.ty_registry.struct_ty_data(ty).fields[field];
                    ty = field_data.ty;
                    bit_offset += field_data.bit_offset;
                }
                mir::PlaceProjection::Index(mir::Operand::Literal(index)) => {
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    bit_offset += ty.bit_size(self.ty_registry).padded() * index.value;
                }
                mir::PlaceProjection::Index(mir::Operand::Local(index)) => {
                    let (index_high, index_low) = self.locals.r16s[*index];
                    let index = self.lir.insert_join(block, index_high, index_low);
                    let byte_offset = bit_offset.bytes();
                    assert_eq!(
                        bit_offset.bits(),
                        u3::new(0),
                        "Cannot index array at unpadded offset"
                    );
                    let addr = match r16 {
                        Some(r16) => {
                            let offset = self
                                .lir
                                .insert_ld16(block, Ld16Source::Literal(byte_offset));
                            let _c = self.lir.insert_add16(block, r16, offset);
                            r16
                        }
                        None => self.offset_global_to_hl(global, byte_offset, block),
                    };
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    let item_byte_size = ty.bit_size(self.ty_registry).padded_bytes();
                    bit_offset = BitCount::new(0);
                    for _ in 0..item_byte_size {
                        let _c = self.lir.insert_add16(block, addr, index);
                        r16 = Some(addr);
                    }
                }
                mir::PlaceProjection::Index(mir::Operand::FnPtr(_)) => {
                    unimplemented!("fn ptr should not be possible to use for indexing an array")
                }
            }
        }
        let byte_offset = bit_offset.bytes();
        let bit_offset = bit_offset.bits();
        let source = match r16 {
            Some(hl) => {
                if byte_offset > 0 {
                    let offset = self
                        .lir
                        .insert_ld16(block, Ld16Source::Literal(byte_offset));
                    let _c = self.lir.insert_add16(block, hl, offset);
                    hl
                } else {
                    hl
                }
            }
            None => self.offset_global_to_hl(global, byte_offset, block),
        };
        (source, bit_offset)
    }

    pub(super) fn offset_global_to_hl(
        &mut self,
        global: mir::GlobalId,
        byte_offset: u16,
        block: BlockId,
    ) -> HLId {
        self.lir
            .insert_ld_hl(block, self.global_to_ld16_source(global, byte_offset))
    }

    pub(super) fn global_to_ld16_source(
        &self,
        global: mir::GlobalId,
        byte_offset: u16,
    ) -> Ld16Source {
        match &self.globals[global] {
            LoweredGlobal::Section(section) => Ld16Source::SectionAddr(SectionAddr {
                section: *section,
                offset: byte_offset,
            }),
            LoweredGlobal::Addr(addr) => Ld16Source::Literal(
                addr.checked_add(byte_offset)
                    .expect("projection offset overflowed addr"),
            ),
        }
    }

    pub(super) fn projected_local_offset(
        &self,
        local: mir::LocalId,
        projections: &[mir::PlaceProjection],
    ) -> BitCount {
        let mut bit_offset = BitCount::new(0);
        let local_ty = self.function.locals[local].ty;
        let mut ty = local_ty;
        for projection in projections {
            match projection {
                mir::PlaceProjection::Field(field) => {
                    let field_data = &self.ty_registry.struct_ty_data(ty).fields[field];
                    ty = field_data.ty;
                    bit_offset += field_data.bit_offset;
                }
                mir::PlaceProjection::Index(mir::Operand::Literal(index)) => {
                    ty = self.ty_registry.array_ty_data(ty).item_ty;
                    bit_offset += ty.bit_size(self.ty_registry).padded() * index.value;
                }
                mir::PlaceProjection::Index(mir::Operand::Local(_)) => {
                    todo!("cannot lower {projection:?} on local")
                }
                mir::PlaceProjection::Index(mir::Operand::FnPtr(_)) => {
                    unimplemented!("fn ptr should not be possible to use for indexing an array")
                }
            }
        }
        bit_offset
    }

    pub(super) fn projected_local_to_r8(
        &self,
        local: mir::LocalId,
        projections: &[mir::PlaceProjection],
    ) -> (R8Id, u3) {
        let bit_offset = self.projected_local_offset(local, projections);
        let local_ty = self.function.locals[local].ty;
        let r8 = if local_ty.bit_size(self.ty_registry).into_u32() <= 8 {
            assert_eq!(bit_offset.bytes(), 0);
            self.locals.r8s[local]
        } else {
            let (high, low) = self.locals.r16s[local];
            match bit_offset.bytes() {
                0 => low,
                1 => high,
                _ => unimplemented!("cannot support bit_offset {bit_offset} > 16-bit"),
            }
        };
        (r8, bit_offset.bits())
    }
}
