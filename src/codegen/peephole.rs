//! Post-register-allocation peephole rewrites.
//!
//! These run once every virtual register has its final color, and only make
//! sense at that point: each rewrite depends on which physical register an
//! operand landed in.
//!
//! From here on the colors are the ground truth. Virtual registers only name
//! the color an operand was given: [`Register::defined`](crate::lir::Register)
//! is no longer maintained and [`CodeSectionData::check_integrity`] no longer
//! holds. Liveness is tracked per physical register ([`Reg`](crate::reg::Reg))
//! instead.
//!
//! The peephole pass is structured as an array of rules. Rules only see
//! consecutive instructions of one block; anything needing a wider view
//! belongs in instruction selection or a MIR optimization.

use crate::lir::{BlockId, CodeSectionData, Instr, Lir, SectionData, Terminator, VirtualReg};
use crate::reg::{Color, RegSet};
use slotmap::SecondaryMap;
use tracing::{instrument, trace};

/// The rules, in the order they are tried at each position.
const RULES: &[&dyn Rule] = &[&AccumulatorRotate];

/// Finalizes register-allocated `lir` for encoding: drops the instructions that only existed to
/// guide the allocator, then applies the peephole rules.
#[instrument(name = "peephole", skip_all)]
pub fn run(lir: &mut Lir) {
    for section in lir.sections.values_mut() {
        if let SectionData::Code(code) = &mut section.data {
            // Sections with no virtual registers are hand-emitted final machine
            // code (e.g. the cartridge header stub pinned at `$0100`). Leaving them
            // alone preserves deliberate instructions such as the header's leading
            // `NOP`.
            if !code.has_registers() {
                continue;
            }
            strip_pseudo_instructions(code);
            apply_rules(code);
        }
    }
}

/// Removes the instructions that emit no machine code once colors are known: the allocator's
/// virtual instructions, and moves whose two sides landed in the same register.
fn strip_pseudo_instructions(code: &mut CodeSectionData) {
    let CodeSectionData {
        blocks,
        r8_registers,
        ..
    } = code;
    for block in blocks.values_mut() {
        block.instructions.retain(|instruction| match instruction {
            Instr::Ld8 { target, source } => {
                r8_registers[*target].color != r8_registers[*source].color
            }
            Instr::Nop
            | Instr::Join { .. }
            | Instr::Split { .. }
            | Instr::MoveZero { .. }
            | Instr::MoveCarry { .. }
            | Instr::LdReturn8 { .. }
            | Instr::LdReturn16 { .. } => false,
            _ => true,
        });
    }
}

/// Sweeps every block once, front to back. At each position the first rule that matches is
/// applied and the same position is tried again, so one rewrite can feed the next.
fn apply_rules(code: &mut CodeSectionData) {
    let live_out = live_out(code);
    for block_id in code.blocks.keys().collect::<Vec<_>>() {
        let mut live_after = live_after(code, block_id, &live_out[block_id]);
        let mut index = 0;
        while index < code.blocks[block_id].instructions.len() {
            let ctx = Context {
                code,
                live_after: &live_after[index..],
            };
            let window = &code.blocks[block_id].instructions[index..];
            let rewrite = RULES.iter().find_map(|rule| {
                let rewrite = rule.rewrite(&ctx, window)?;
                trace!(rule = rule.name(), ?rewrite, "rewrite");
                Some(rewrite)
            });
            let Some(rewrite) = rewrite else {
                index += 1;
                continue;
            };
            code.blocks[block_id]
                .instructions
                .splice(index..index + rewrite.len, rewrite.instrs);
            // Only this block is recomputed. The live-out sets of the other blocks may now
            // list a register the rewrite stopped reading, which is merely conservative.
            live_after = self::live_after(code, block_id, &live_out[block_id]);
        }
    }
}

/// A peephole rewrite.
trait Rule: Sync {
    /// Identifies the rule in traces.
    fn name(&self) -> &'static str;

    /// The replacement, if any, for the instructions at the start of `window`, which runs from
    /// the position being looked at to the end of its block.
    ///
    /// The replacement must not match the rule again, must not read a register the replaced
    /// instructions did not read, and may only write a register they did not write if it is
    /// [dead](Context::dead_after) after them.
    fn rewrite(&self, ctx: &Context, window: &[Instr]) -> Option<Rewrite>;
}

/// Replaces the first `len` instructions of a [`Rule`]'s window with `instrs`.
#[derive(Debug)]
struct Rewrite {
    len: usize,
    instrs: Vec<Instr>,
}

/// What a [`Rule`] can ask about the instructions in its window.
struct Context<'a> {
    code: &'a CodeSectionData,
    /// The registers live after each instruction of the window.
    live_after: &'a [RegSet],
}

impl Context<'_> {
    /// The color `reg` was allocated.
    fn color(&self, reg: impl Into<VirtualReg>) -> Color {
        let reg = reg.into();
        self.code
            .register_data(reg)
            .color
            .unwrap_or_else(|| panic!("{reg:?} has no color after register allocation"))
    }

    /// Whether nothing reads `color` after the first `len` instructions of the window before
    /// it is next written.
    fn dead_after(&self, len: usize, color: Color) -> bool {
        (self.live_after[len - 1].clone() & color.regs()).is_empty()
    }
}

/// The physical registers `regs` were allocated.
fn regs(code: &CodeSectionData, regs: impl Iterator<Item = VirtualReg>) -> RegSet {
    let mut set = RegSet::empty();
    for reg in regs {
        if let Some(color) = code.register_data(reg).color {
            set |= color.regs();
        }
    }
    set
}

/// The registers live before `instruction`, given those live after it.
fn live_before(code: &CodeSectionData, instruction: &Instr, mut live: RegSet) -> RegSet {
    let defs = regs(code, instruction.register_defs());
    live.retain(|reg| !defs.contains(reg));
    live | regs(code, instruction.register_uses())
}

/// The registers `terminator` reads, including those something outside the function reads once
/// it leaves.
fn terminator_uses(code: &CodeSectionData, terminator: &Terminator) -> RegSet {
    let exit = match terminator {
        // The return value is in `A` or `BC`. The virtual registers do not record which, if
        // either, the function returns.
        Terminator::Ret | Terminator::RetIf { .. } => Color::A.regs() | Color::BC.regs(),
        // The interrupted code expects every register back.
        Terminator::Reti => Color::Call.regs(),
        // A callee takes nothing in registers, and what the caller reads after a call returns
        // is live into the return block.
        Terminator::Call { .. }
        | Terminator::CallIf { .. }
        | Terminator::CallR16 { .. }
        | Terminator::Jp { .. }
        | Terminator::JpFn { .. }
        | Terminator::JpIf { .. }
        | Terminator::Halt { .. } => RegSet::empty(),
    };
    exit | regs(code, terminator.register_uses())
}

/// The registers live after the terminator of each block.
fn live_out(code: &CodeSectionData) -> SecondaryMap<BlockId, RegSet> {
    let mut live_in: SecondaryMap<BlockId, RegSet> = SecondaryMap::new();
    let mut live_out: SecondaryMap<BlockId, RegSet> = SecondaryMap::new();
    for block_id in code.blocks.keys() {
        live_in.insert(block_id, RegSet::empty());
        live_out.insert(block_id, RegSet::empty());
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (block_id, block) in &code.blocks {
            let mut out = RegSet::empty();
            for next in block.next_blocks() {
                out |= live_in[next].clone();
            }
            let mut live = out.clone() | terminator_uses(code, &block.terminator);
            for instruction in block.instructions.iter().rev() {
                live = live_before(code, instruction, live);
            }
            if out != live_out[block_id] || live != live_in[block_id] {
                live_out.insert(block_id, out);
                live_in.insert(block_id, live);
                changed = true;
            }
        }
    }
    live_out
}

/// The registers live after each instruction of `block_id`, given those live after its
/// terminator.
fn live_after(code: &CodeSectionData, block_id: BlockId, live_out: &RegSet) -> Vec<RegSet> {
    let block = &code.blocks[block_id];
    let mut live = live_out.clone() | terminator_uses(code, &block.terminator);
    let mut live_after = Vec::with_capacity(block.instructions.len());
    for instruction in block.instructions.iter().rev() {
        live_after.push(live.clone());
        live = live_before(code, instruction, live);
    }
    live_after.reverse();
    live_after
}

/// Rewrites CB-prefixed rotates of `A` into the 1-byte accumulator forms
/// (`RLA`/`RLCA`/`RRA`/`RRCA`).
///
/// The accumulator forms are a byte shorter and idiomatic, but they hard-clear
/// `Z` instead of setting it to `result == 0`, so this only fires when the zero
/// flag is dead after the rotate. The `z` register is kept on the rewritten
/// instruction, as it still models the write to `Fz`.
struct AccumulatorRotate;

impl Rule for AccumulatorRotate {
    fn name(&self) -> &'static str {
        "accumulator_rotate"
    }

    fn rewrite(&self, ctx: &Context, window: &[Instr]) -> Option<Rewrite> {
        let (source, instr) = match window[0] {
            Instr::Rl { z, c, source, c_in } => (
                source,
                Instr::Rla {
                    z,
                    c,
                    source: source.downcast_to_a_without_move(),
                    c_in,
                },
            ),
            Instr::Rlc { z, c, source } => (
                source,
                Instr::Rlca {
                    z,
                    c,
                    source: source.downcast_to_a_without_move(),
                },
            ),
            Instr::Rr { z, c, source, c_in } => (
                source,
                Instr::Rra {
                    z,
                    c,
                    source: source.downcast_to_a_without_move(),
                    c_in,
                },
            ),
            Instr::Rrc { z, c, source } => (
                source,
                Instr::Rrca {
                    z,
                    c,
                    source: source.downcast_to_a_without_move(),
                },
            ),
            _ => return None,
        };
        (ctx.color(source) == Color::A && ctx.dead_after(1, Color::Fz)).then(|| Rewrite {
            len: 1,
            instrs: vec![instr],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Identifier;
    use crate::lir::{Block, FlagId, Register, ZeroId};
    use slotmap::{SlotMap, SparseSecondaryMap};

    /// Builds a one-block function whose sole instruction is `RLC source`, coloring `source`
    /// with `rotate_color`. The block's terminator is a bare `RET`; callers that want the
    /// rotate's `z` flag to be live overwrite it.
    fn code_with_single_rlc(rotate_color: Color) -> (CodeSectionData, BlockId, ZeroId) {
        let mut code = CodeSectionData {
            region: None,
            addr: None,
            functions: SparseSecondaryMap::new(),
            blocks: SlotMap::with_key(),
            zero_registers: SlotMap::with_key(),
            carry_registers: SlotMap::with_key(),
            r8_registers: SlotMap::with_key(),
            r16_registers: SlotMap::with_key(),
        };
        let r8 = code.r8_registers.insert(Register {
            color: Some(rotate_color),
            ..Register::default()
        });
        let z = code.zero_registers.insert(Register {
            color: Some(Color::Fz),
            ..Register::default()
        });
        let c = code.carry_registers.insert(Register {
            color: Some(Color::Fc),
            ..Register::default()
        });
        let block = code.blocks.insert(Block {
            instructions: vec![Instr::Rlc { z, c, source: r8 }],
            terminator: Terminator::Ret,
            previous: Vec::new(),
        });
        code.functions.insert(block, Identifier::from("f"));
        (code, block, z)
    }

    #[test]
    fn accumulator_rotate_rewrites_a_rotate_with_dead_zero() {
        let (mut code, block, z) = code_with_single_rlc(Color::A);
        apply_rules(&mut code);
        assert!(
            matches!(
                code.blocks[block].instructions[0],
                Instr::Rlca { z: rewritten_z, .. } if rewritten_z == z
            ),
            "an A-colored RLC with a dead zero should become RLCA, keeping its z, got {:?}",
            code.blocks[block].instructions[0]
        );
    }

    #[test]
    fn accumulator_rotate_keeps_cb_form_when_zero_is_live() {
        let (mut code, block, z) = code_with_single_rlc(Color::A);
        code.blocks[block].terminator = Terminator::RetIf {
            source: FlagId::Zero(z),
            negate: false,
            else_block: block,
        };
        apply_rules(&mut code);
        assert!(
            matches!(code.blocks[block].instructions[0], Instr::Rlc { .. }),
            "an RLC whose zero flag feeds a RetIf must keep the CB form, got {:?}",
            code.blocks[block].instructions[0]
        );
    }

    #[test]
    fn accumulator_rotate_keeps_cb_form_for_non_a_register() {
        let (mut code, block, _z) = code_with_single_rlc(Color::B);
        apply_rules(&mut code);
        assert!(
            matches!(code.blocks[block].instructions[0], Instr::Rlc { .. }),
            "a non-A-colored RLC must keep the CB form, got {:?}",
            code.blocks[block].instructions[0]
        );
    }
}
