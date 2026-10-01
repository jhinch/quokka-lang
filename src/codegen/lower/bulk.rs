use super::InstrCtx;
use crate::lir::{BlockId, Ld16Source};
use crate::mir;
use crate::num::u3;

impl InstrCtx<'_> {
    /// Lowers `Instr::Transfer`/`Instr::Fill` to a single [`lir::Instr::BulkCopy`]/
    /// [`lir::Instr::BulkFill`] macro instruction (see those types for why this is one atomic
    /// LIR instruction rather than a real multi-block loop). The analyzer only ever emits these
    /// with empty projections (v1 only supports whole-global copy/fill), so addressing
    /// is always "whole global, offset 0".
    pub(super) fn generate_from_transfer_or_fill(
        &mut self,
        instruction: &mir::Instr,
        block: BlockId,
    ) {
        match instruction {
            mir::Instr::Transfer {
                target,
                target_projections,
                source,
                source_projections,
            } => {
                let (target_addr, bits) =
                    self.projected_global_to_addr(*target, target_projections, block);
                assert_eq!(bits, u3::new(0), "misaligned ty");
                let target_addr = {
                    let target_addr = target_addr.upcast_to_r16_without_move();
                    let (target_addr_high, target_addr_low) =
                        self.lir.insert_split(block, target_addr);
                    let target_addr_low = self.lir.insert_ld8(block, target_addr_low);
                    let target_addr_high = self.lir.insert_ld8(block, target_addr_high);
                    self.lir
                        .insert_join(block, target_addr_high, target_addr_low)
                };
                let (source_addr, bits) =
                    self.projected_global_to_addr(*source, source_projections, block);
                assert_eq!(bits, u3::new(0), "misaligned ty");

                let ty = self.projected_ty(*target, target_projections);
                let count = self.lir.insert_ld16(
                    block,
                    Ld16Source::Literal(ty.bit_size(self.ty_registry).padded_bytes()),
                );
                let (_z, _c) = self
                    .lir
                    .insert_bulk_copy(block, target_addr, source_addr, count);
            }
            mir::Instr::Fill {
                target,
                projections,
                value,
                count,
            } => {
                let (target_addr, bits) =
                    self.projected_global_to_addr(*target, projections, block);
                assert_eq!(bits, u3::new(0), "misaligned ty");
                let value = self.lir.insert_ld8_value(block, *value);
                let value = self.lir.insert_ld_a(block, value);
                let count = self
                    .lir
                    .insert_ld16(block, Ld16Source::Literal(count.get()));
                let (_z, _c) = self.lir.insert_bulk_fill(block, target_addr, count, value);
            }
            _ => unreachable!("generate_from_transfer_or_fill called with {instruction:?}"),
        }
    }
}
