use crate::mir;
use crate::ty::Ty;
use slotmap::SecondaryMap;

/// How a MIR instruction is lowered to LIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Selection {
    /// Lowered on its own, through the per-width `generate_from_*` paths.
    Instr,
    /// Not lowered: every user of its target was selected in a way that does not read it.
    Covered,
    /// `target = (source & (1 << bit)) != 0`, or `== 0` when `condition` is
    /// [`mir::Condition::Zero`], lowered to a test of that bit of `source` instead of an `AND`
    /// followed by a compare.
    BitTest {
        target: mir::LocalId,
        source: mir::LocalId,
        /// The tested bit, counted from the least significant bit of `source`.
        bit: u8,
        condition: mir::Condition,
    },
    /// `target = source + delta` for a small constant `delta`, lowered to `INC`/`DEC` steps of
    /// a copy of `source` instead of an addition through the accumulator.
    Step {
        target: mir::LocalId,
        source: mir::LocalId,
        /// The number of steps to take: increments when positive, decrements when negative.
        delta: i8,
    },
}

/// The [`Selection`] of every instruction of a function.
pub(super) struct Plan {
    /// For each block, a selection for each of its instructions.
    selections: SecondaryMap<mir::BlockId, Vec<Selection>>,
}

impl Plan {
    /// Selects how each instruction of `function` is lowered.
    ///
    /// Instructions are visited in reverse, so that every user of a local is selected before the
    /// instruction defining it. A selection that does not read one of its instruction's operands
    /// releases that use, and a pure definition left with no uses is [`Selection::Covered`].
    pub(super) fn new(function: &mir::Function) -> Self {
        let blocks: Vec<mir::BlockId> = function.traverse_blocks().collect();
        let mut planner = Planner {
            function,
            uses: function.count_uses(),
            selections: blocks
                .iter()
                .map(|&block| {
                    let instructions = &function.blocks[block].instructions;
                    (block, vec![Selection::Instr; instructions.len()])
                })
                .collect(),
        };
        // a block is traversed after the blocks dominating it, so the reverse order reaches the
        // users of a local before its definition
        for &block in blocks.iter().rev() {
            for index in (0..function.blocks[block].instructions.len()).rev() {
                let instruction = &function.blocks[block].instructions[index];
                if planner.selections[block][index] == Selection::Covered {
                    for source in instruction.source_locals() {
                        planner.release(source);
                    }
                } else {
                    planner.selections[block][index] = planner.select(instruction);
                }
            }
        }
        Plan {
            selections: planner.selections,
        }
    }

    /// The selection of each instruction of `block`, in instruction order.
    pub(super) fn block(&self, block: mir::BlockId) -> &[Selection] {
        &self.selections[block]
    }
}

struct Planner<'a> {
    function: &'a mir::Function,
    /// How many reads of each local are left among the instructions selected so far.
    uses: SecondaryMap<mir::LocalId, u32>,
    selections: SecondaryMap<mir::BlockId, Vec<Selection>>,
}

impl<'a> Planner<'a> {
    fn select(&mut self, instruction: &mir::Instr) -> Selection {
        match instruction {
            mir::Instr::Eq {
                target,
                left,
                right,
            } => self.select_bit_test(*target, *left, right, mir::Condition::Zero),
            mir::Instr::Ne {
                target,
                left,
                right,
            } => self.select_bit_test(*target, *left, right, mir::Condition::NotZero),
            mir::Instr::Add {
                target,
                left,
                right,
            } => self.select_step(*target, *left, right, false),
            mir::Instr::Sub {
                target,
                left,
                right,
            } => self.select_step(*target, *left, right, true),
            _ => None,
        }
        .unwrap_or(Selection::Instr)
    }

    /// Matches `target = left + right` (or `left - right` when `subtract`) with a constant
    /// `right` small enough that stepping `left` is cheaper than adding to it.
    fn select_step(
        &mut self,
        target: mir::LocalId,
        left: mir::LocalId,
        right: &mir::Operand,
        subtract: bool,
    ) -> Option<Selection> {
        let constant = self.constant(right)?;
        let constant = if subtract {
            constant.wrapping_neg()
        } else {
            constant
        };
        let delta = match self.function.locals[target].ty {
            Ty::U8 | Ty::I8 => match constant & 0xFF {
                1 => 1,
                2 => 2,
                0xFF => -1,
                0xFE => -2,
                _ => return None,
            },
            // a 16-bit step branches over the carry into the high byte, so a second step would
            // cost as much as the byte-pair addition
            Ty::U16 | Ty::I16 => match constant & 0xFFFF {
                1 => 1,
                0xFFFF => -1,
                _ => return None,
            },
            _ => return None,
        };
        if let mir::Operand::Local(right) = right {
            self.release(*right);
        }
        Some(Selection::Step {
            target,
            source: left,
            delta,
        })
    }

    /// Matches `target = (source & mask) == 0` (or `!= 0`) with a single-bit `mask`, where `left`
    /// is the result of the `&`. `condition` is the state of the bit for which `target` is true.
    fn select_bit_test(
        &mut self,
        target: mir::LocalId,
        left: mir::LocalId,
        right: &mir::Operand,
        condition: mir::Condition,
    ) -> Option<Selection> {
        if self.constant(right)? != 0 {
            return None;
        }
        let Some(mir::Instr::BitAnd {
            left: source,
            right: mask,
            ..
        }) = self.def(left)
        else {
            return None;
        };
        let mask = match self.function.locals[left].ty {
            Ty::U8 | Ty::I8 => self.constant(mask)? & 0xFF,
            Ty::U16 | Ty::I16 => self.constant(mask)? & 0xFFFF,
            _ => return None,
        };
        if !mask.is_power_of_two() {
            return None;
        }
        self.release(left);
        if let mir::Operand::Local(right) = right {
            self.release(*right);
        }
        Some(Selection::BitTest {
            target,
            source: *source,
            #[allow(clippy::cast_possible_truncation)]
            bit: mask.trailing_zeros() as u8,
            condition,
        })
    }

    /// The instruction defining `local`, wherever in the function it is. `None` for a local
    /// without exactly one definition, such as the return local.
    fn def(&self, local: mir::LocalId) -> Option<&'a mir::Instr> {
        let (block, index) = self.def_location(local)?;
        Some(&self.function.blocks[block].instructions[index])
    }

    fn def_location(&self, local: mir::LocalId) -> Option<(mir::BlockId, usize)> {
        let mut defined = self.function.locals[local].defined.iter();
        match (defined.next(), defined.next()) {
            (Some((block, &index)), None) => Some((block, index)),
            _ => None,
        }
    }

    /// The value of `operand` if it is known at compile time: a literal, or a local defined by a
    /// `Copy` of a literal.
    fn constant(&self, operand: &mir::Operand) -> Option<u32> {
        match operand {
            mir::Operand::Literal(literal) => Some(literal.value),
            mir::Operand::Local(local) => match self.def(*local)? {
                mir::Instr::Copy {
                    source: mir::Operand::Literal(literal),
                    ..
                } => Some(literal.value),
                _ => None,
            },
            mir::Operand::FnPtr(_) => None,
        }
    }

    /// Records that the instruction being selected does not read `local`, although it is one of
    /// its operands. The definition of `local` is covered once no reads of it are left, provided
    /// it is pure.
    fn release(&mut self, local: mir::LocalId) {
        let uses = &mut self.uses[local];
        *uses -= 1;
        if *uses > 0 || self.function.locals[local].is_return {
            return;
        }
        if let Some((block, index)) = self.def_location(local)
            && self.function.blocks[block].instructions[index].is_pure()
        {
            self.selections[block][index] = Selection::Covered;
        }
    }
}
