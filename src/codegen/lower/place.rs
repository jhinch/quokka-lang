use super::InstrCtx;
use crate::attr;
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
                    let (index_high, index_low) = self.pair(*index);
                    let index = self.lir.insert_join(block, index_high, index_low);
                    let byte_offset = bit_offset.bytes();
                    assert_eq!(
                        bit_offset.bits(),
                        u3::new(0),
                        "Cannot index array at unpadded offset"
                    );
                    let addr = match r16 {
                        Some(r16) => {
                            self.advance_addr(global, r16, byte_offset, block);
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
                self.advance_addr(global, hl, byte_offset, block);
                hl
            }
            None => self.offset_global_to_hl(global, byte_offset, block),
        };
        (source, bit_offset)
    }

    /// Advances `addr`, an address within `global`, by `byte_offset` bytes.
    fn advance_addr(
        &mut self,
        global: mir::GlobalId,
        addr: HLId,
        byte_offset: u16,
        block: BlockId,
    ) {
        if byte_offset == 0 {
            return;
        }
        // `INC HL` is 1 byte and 8 cycles against 4 bytes and 20 cycles for `LD r16,n16` and
        // `ADD HL,r16`, so it only wins for the first two steps
        if byte_offset <= 2 && self.global_avoids_oam(global) {
            for _ in 0..byte_offset {
                self.lir
                    .insert_inc16(block, addr.upcast_to_r16_without_move());
            }
        } else {
            let offset = self
                .lir
                .insert_ld16(block, Ld16Source::Literal(byte_offset));
            let _c = self.lir.insert_add16(block, addr, offset);
        }
    }

    /// Whether no address within `global` is in `$FE00-$FEFF`. `INC r16` and `DEC r16` corrupt
    /// OAM when the register holds an address in that range, so they can only step an address
    /// into a global placed outside of it.
    fn global_avoids_oam(&self, global: mir::GlobalId) -> bool {
        match &self.mir.globals[global].attrs {
            mir::GlobalAttributes::Rom(_) => true,
            mir::GlobalAttributes::Ram(attrs) => match attrs.region {
                attr::RamRegionKind::Oam => false,
                attr::RamRegionKind::Auto => !matches!(attrs.address, Some(0xFE00..=0xFEFF)),
                attr::RamRegionKind::WRam0
                | attr::RamRegionKind::VRam
                | attr::RamRegionKind::HRam
                | attr::RamRegionKind::WRamX
                | attr::RamRegionKind::SRam => true,
            },
        }
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
            self.byte(local)
        } else {
            let (high, low) = self.pair(local);
            match bit_offset.bytes() {
                0 => low,
                1 => high,
                _ => unimplemented!("cannot support bit_offset {bit_offset} > 16-bit"),
            }
        };
        (r8, bit_offset.bits())
    }
}
