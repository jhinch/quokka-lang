//! # Quokka Lang's Middle Intermediate Representation (MIR)
//!
//! MIR ([`Mir`]) represents a Quokka program as a collection of control flow
//! graphs in static single assignment (SSA) form. The instructions ([`Instr`]) within
//! a basic block ([`Block`]) represent language concepts rather than machine level
//! concepts like [`crate::lir::Lir`] does.
//!
//! Once built, MIR [`Function`] can be [evaluated](crate::consteval::evaluate) provided it depends
//! only on constants and can be used to evaluate the value of a constant.
//!

use crate::ast::{Identifier, IntegerLiteral, SpannedIdentifier};
use crate::attr::{ConstDeclAttributes, FnDeclAttributes, GlobalDeclAttributes};
use crate::multi::{MultiError, MultiResultBuilder};
use crate::ty::{Ty, TyRegistry};
use indexmap::{IndexMap, IndexSet};
use slotmap::sparse_secondary::Entry;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap, new_key_type};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::ops::Not;

new_key_type! {
    /// Identifies a [`Global`] within a [`Mir`]'s global table.
    pub struct GlobalId;
}
new_key_type! {
    /// Identifies a [`Function`] within a [`Mir`]'s function table.
    pub struct FunctionId;
}
new_key_type! {
    /// Identifies a [`Block`] within a [`Function`]'s control flow graph.
    pub struct BlockId;
}
new_key_type! {
    /// Identifies a [`Local`] (an SSA value) within a [`Function`].
    pub struct LocalId;
}

impl LocalId {
    #[must_use]
    pub fn display<'a>(self, function: &'a Function) -> impl fmt::Display + 'a {
        function.locals[self].display(self)
    }

    #[must_use]
    pub fn display_with_ty<'a>(
        self,
        function: &'a Function,
        ty_registry: &'a TyRegistry,
    ) -> impl fmt::Display + 'a {
        function.locals[self].display_with_ty(self, ty_registry)
    }
}

impl GlobalId {
    #[must_use]
    pub fn display<'a>(self, mir: &'a Mir) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| write!(f, "{}", mir.globals[self].name.data))
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".bb{}", slotmap::Key::data(self).as_ffi() & 0xffff_ffff)
    }
}

/// Middle Intermediate Representation (MIR)
///
/// A complete MIR program: every global (constant or mutable) and every function
/// that make up a compiled Quokka program.
#[derive(Debug, Clone, Default)]
pub struct Mir {
    /// All globals (constants and mutable globals) defined by the program.
    pub globals: SlotMap<GlobalId, Global>,
    /// All functions defined by the program.
    pub functions: SlotMap<FunctionId, Function>,
}

impl Mir {
    #[must_use]
    pub fn display<'a>(&'a self, ty_registry: &'a TyRegistry) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            for global in self.globals.values() {
                write!(f, "{}", global.display(ty_registry))?;
            }
            writeln!(f)?;
            for function in self.functions.values() {
                if function.attrs != FnDeclAttributes::default() {
                    writeln!(f, "@{:?}", function.attrs)?;
                }
                writeln!(
                    f,
                    "{} -> {}:",
                    function.name,
                    function.return_ty.display(ty_registry)
                )?;
                for block in function.traverse_blocks() {
                    writeln!(f, "  {block}:")?;
                    let block = &function.blocks[block];
                    for instruction in &block.instructions {
                        write!(f, "    ")?;
                        if let Some(target) = instruction.target_local() {
                            write!(f, "{} = ", target.display_with_ty(function, ty_registry))?;
                        }
                        match instruction {
                            Instr::Undefined { target: _ } => write!(f, "undefined"),
                            Instr::Alloc { target: _ } => write!(f, "alloc"),
                            Instr::ReturnValue { target: _ } => write!(f, "returnvalue"),
                            Instr::Phi { target: _, sources } => {
                                write!(f, "phi")?;
                                let mut sep = " ";
                                let mut sources = sources
                                    .iter()
                                    .map(|(block, source)| (block, *source))
                                    .collect::<Vec<_>>();
                                sources.sort_by_key(|(block, _)| *block);
                                for (block, source) in sources.iter().copied() {
                                    write!(f, "{sep}{} <- {}", source.display(function), block)?;
                                    sep = ", ";
                                }
                                Ok(())
                            }
                            Instr::Load {
                                target: _,
                                source,
                                projections,
                            } => {
                                write!(f, "load {}", source.display(self))?;
                                for projection in projections {
                                    write!(f, "{}", projection.display(self, function))?;
                                }
                                Ok(())
                            }
                            Instr::Store {
                                target,
                                projections,
                                source,
                            } => {
                                write!(f, "store {}", target.display(self))?;
                                for projection in projections {
                                    write!(f, "{}", projection.display(self, function))?;
                                }
                                write!(f, " = {}", source.display(self, function))
                            }
                            Instr::Read {
                                target: _,
                                source,
                                projections,
                            } => {
                                write!(f, "read {}", source.display(function))?;
                                for projection in projections {
                                    write!(f, "{}", projection.display(self, function))?;
                                }
                                Ok(())
                            }
                            Instr::Write {
                                target: _,
                                left,
                                projections,
                                right,
                            } => {
                                write!(f, "write {}", left.display(function))?;
                                for projection in projections {
                                    write!(f, "{}", projection.display(self, function))?;
                                }
                                write!(f, " = {}", right.display(self, function))
                            }
                            Instr::Copy { target: _, source } => {
                                write!(f, "copy {}", source.display(self, function))
                            }
                            Instr::Not { target: _, source } => {
                                write!(f, "not {}", source.display(function))
                            }
                            Instr::Neg { target: _, source } => {
                                write!(f, "neg {}", source.display(function))
                            }
                            Instr::Eq {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "eq {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Ne {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "ne {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Lt {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "lt {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Le {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "le {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Ge {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "ge {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Gt {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "gt {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::BitAnd {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "bitand {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::BitOr {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "bitor {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::BitXor {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "bitxor {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::RotateLeft { target: _, source } => {
                                write!(f, "rotateleft {}", source.display(function),)
                            }
                            Instr::RotateRight { target: _, source } => {
                                write!(f, "rotateright {}", source.display(function))
                            }
                            Instr::ShiftLeft { target: _, source } => {
                                write!(f, "shiftleft {}", source.display(function))
                            }
                            Instr::ShiftRight { target: _, source } => {
                                write!(f, "shiftright {}", source.display(function))
                            }
                            Instr::Add {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "add {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                            Instr::Sub {
                                target: _,
                                left,
                                right,
                            } => {
                                write!(
                                    f,
                                    "sub {}, {}",
                                    left.display(function),
                                    right.display(self, function)
                                )
                            }
                        }?;
                        writeln!(f)?;
                    }
                    write!(f, "    ")?;
                    match &block.terminator {
                        Terminator::Return => write!(f, "return"),
                        Terminator::Call {
                            callee,
                            return_block,
                        } => {
                            write!(f, "call {}, {return_block}", callee.display(self, function))
                        }
                        Terminator::CallIf {
                            callee,
                            source,
                            condition,
                            return_block,
                            else_block,
                        } => {
                            write!(
                                f,
                                "callif {}, {} {condition}, {return_block}, {else_block}",
                                source.display(function),
                                callee.display(self, function)
                            )
                        }
                        Terminator::Jump { target_block } => {
                            write!(f, "jump {target_block}")
                        }
                        Terminator::JumpIf {
                            source,
                            condition,
                            then_block,
                            else_block,
                        } => {
                            write!(
                                f,
                                "jumpif {} {condition}, {then_block}, {else_block}",
                                source.display(function)
                            )
                        }
                        Terminator::ReturnIf {
                            source,
                            condition,
                            else_block,
                        } => {
                            write!(
                                f,
                                "returnif {} {condition}, {else_block}",
                                source.display(function)
                            )
                        }
                    }?;
                    writeln!(f)?;
                }
                writeln!(f)?;
            }
            Ok(())
        })
    }
}

/// MIR function represented as a control flow graph in static single assignment form
#[derive(Debug, Clone, Default)]
pub struct Function {
    pub name: Identifier,
    pub attrs: FnDeclAttributes,
    pub return_ty: Ty,
    /// The local that holds the function's return value, if any.
    pub return_local: Option<LocalId>,
    /// All basic blocks belonging to the function's control flow graph.
    pub blocks: SlotMap<BlockId, Block>,
    /// All SSA locals (values) used by the function.
    pub locals: SlotMap<LocalId, Local>,
    /// The block where execution of the function begins.
    pub start_block: BlockId,
    /// The blocks that may terminate execution of the function (i.e. those ending in
    /// [`Terminator::Return`] or [`Terminator::ReturnIf`]).
    pub end_blocks: IndexSet<BlockId>,
}

impl Function {
    /// Iterates the function's blocks in breadth-first order starting from
    /// [`Function::start_block`], visiting each reachable block exactly once.
    pub fn traverse_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        std::iter::once(self.start_block).chain(
            self.downstream_blocks(self.start_block)
                .filter(|&block| self.start_block != block),
        )
    }

    pub fn downstream_blocks(&self, previous: BlockId) -> impl Iterator<Item = BlockId> + '_ {
        let mut processed_blocks = SecondaryMap::new();
        let mut block_queue = VecDeque::new();
        block_queue.extend(self.blocks[previous].next_blocks());
        std::iter::from_fn(move || {
            while let Some(block) = block_queue.pop_front() {
                if processed_blocks.insert(block, ()).is_none() {
                    block_queue.extend(self.blocks[block].next_blocks());
                    return Some(block);
                }
            }
            None
        })
    }

    pub fn upstream_blocks(&self, next: BlockId) -> impl Iterator<Item = BlockId> + '_ {
        let mut processed_blocks = SecondaryMap::new();
        let mut block_queue = VecDeque::new();
        block_queue.extend(self.blocks[next].previous_blocks());
        std::iter::from_fn(move || {
            while let Some(block) = block_queue.pop_front() {
                if processed_blocks.insert(block, ()).is_none() {
                    block_queue.extend(self.blocks[block].previous_blocks());
                    return Some(block);
                }
            }
            None
        })
    }

    pub fn remove_dangling(&mut self) {
        let all_blocks = self.blocks.keys().collect::<IndexSet<_>>();
        let reachable_blocks = self.traverse_blocks().collect::<IndexSet<_>>();
        for block in all_blocks.difference(&reachable_blocks) {
            self.blocks.remove(*block);
        }
        let all_locals = self.locals.keys().collect::<IndexSet<_>>();
        let reachable_locals = reachable_blocks
            .iter()
            .copied()
            .flat_map(|block_id| &self.blocks[block_id].instructions)
            .flat_map(|instruction| instruction.target_local())
            .collect::<IndexSet<_>>();
        for local in all_locals.difference(&reachable_locals) {
            self.locals.remove(*local);
        }
    }

    pub fn check_integrity(&self) -> Result<(), MultiError> {
        let mut result_builder = MultiResultBuilder::default();
        // check blocks first
        for (block_id, block) in &self.blocks {
            if block.previous.is_empty() && self.start_block != block_id {
                result_builder.add_error(miette::MietteDiagnostic::new(format!(
                    "dangling block {block_id:?}"
                )));
            }
            for previous_id in block.previous.iter().copied() {
                match self.blocks.get(previous_id) {
                    None => {
                        result_builder.add_error(miette::MietteDiagnostic::new(format!(
                            "block {block_id:?} has unknown previous {previous_id:?}"
                        )));
                    }
                    Some(previous) => {
                        if !previous
                            .next_blocks()
                            .any(|previous_next_block_id| previous_next_block_id == block_id)
                        {
                            result_builder.add_error(miette::MietteDiagnostic::new(format!(
                                "mismatch previous/next blocks {previous_id:?} and {block_id:?}"
                            )));
                        }
                    }
                }
            }
        }
        for block_id in self.end_blocks.iter().copied() {
            if !self.blocks.contains_key(block_id) {
                result_builder.add_error(miette::MietteDiagnostic::new(format!(
                    "unknown end block {block_id:?}"
                )));
            }
        }
        result_builder.take().finish(()).into_result()?;
        // then local definition and types
        for (local_id, local) in &self.locals {
            if matches!(local.ty, Ty::UNRESOLVED | Ty::ERROR | Ty::UNIT | Ty::NEVER) {
                result_builder.add_error(miette::MietteDiagnostic::new(format!(
                    "{local_id:?} has unsupported type {:?}",
                    local.ty
                )));
            }
            if local.defined.is_empty() {
                result_builder.add_error(miette::MietteDiagnostic::new(format!(
                    "dangling local {local_id:?}"
                )));
            } else {
                if local.defined.len() > 1 && Some(local_id) != self.return_local {
                    result_builder.add_error(miette::MietteDiagnostic::new(format!(
                        " local {local_id:?} assigned multiple times {:?}",
                        local.defined
                    )));
                }
                for (block_id, &index) in &local.defined {
                    match self.blocks.get(block_id) {
                        None => {
                            result_builder.add_error(miette::MietteDiagnostic::new(format!(
                                "local {local_id:?} defined in unknown block {block_id:?}"
                            )));
                        }
                        Some(block) => match block.instructions.get(index) {
                            None => {
                                result_builder.add_error(miette::MietteDiagnostic::new(format!(
                                    "local {local_id:?} instruction {index} in block {block_id:?}"
                                )));
                            }
                            Some(instruction) => {
                                if instruction.target_local() != Some(local_id) {
                                    result_builder.add_error(miette::MietteDiagnostic::new(format!("local {local_id:?} not defined in instruction {instruction:?} in block {block_id:?}")));
                                }
                            }
                        },
                    }
                }
            }
        }
        result_builder.take().finish(()).into_result()?;
        // check instruction integrity
        let block_to_upstream_blocks = self
            .blocks
            .keys()
            .map(|block_id| {
                (
                    block_id,
                    self.upstream_blocks(block_id).collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        for (block_id, block) in &self.blocks {
            for (index, instruction) in block.instructions.iter().enumerate() {
                match instruction {
                    Instr::Undefined { .. } => {
                        result_builder.add_error(miette::MietteDiagnostic::new(format!(
                            "Undefined instruction in block {block_id:?} at index {index}"
                        )));
                    }
                    Instr::ReturnValue { .. } => {
                        if index != 0
                            || block.previous.len() != 1
                            || !matches!(self.blocks[block.previous.first().copied().unwrap()].terminator, Terminator::Call { return_block, .. } | Terminator::CallIf { return_block, .. } if return_block == block_id)
                        {
                            result_builder.add_error(miette::MietteDiagnostic::new(format!("ReturnValue instruction must be the next instruction after a call. Found {block_id:?} at index {index} with previous = {:?}", block.previous.iter().copied().map(|previous| &self.blocks[previous].terminator).collect::<Vec<_>>())));
                        }
                    }
                    Instr::Phi { sources, .. } => {
                        let source_block_ids = sources.keys().collect::<IndexSet<_>>();
                        if block.previous != source_block_ids {
                            result_builder.add_error(miette::MietteDiagnostic::new(format!("Phi instruction found in {block_id:?} at index {index} with is missing sources. Expected {:?}, actual {:?}", block.previous, source_block_ids)));
                        }
                        for (previous_block_id, source_id) in sources {
                            match self.locals.get(*source_id).and_then(|source| source.defined.iter().next()) {
                                None => result_builder.add_error(miette::MietteDiagnostic::new(format!("instruction {instruction:?} in {block_id:?} at index {index} uses dangling source {source_id:?}"))),
                                Some((source_block_id, &source_index)) => {
                                    if previous_block_id != source_block_id && !block_to_upstream_blocks[&previous_block_id].contains(&source_block_id) {
                                    result_builder.add_error(miette::MietteDiagnostic::new(format!("instruction {instruction:?} in {block_id:?} at index {index} uses {source_id:?} before assigned. Source defined in block {source_block_id:?} at index {source_index}")));
                                    }
                                }
                            }
                        }
                    }
                    instruction => {
                        for source_id in instruction.source_locals() {
                            match self.locals.get(source_id).and_then(|source| source.defined.iter().next()) {
                                None => result_builder.add_error(miette::MietteDiagnostic::new(format!("instruction {instruction:?} in {block_id:?} at index {index} uses dangling source {source_id:?}"))),
                                Some((source_block_id, &source_index)) => {
                                    if block_id == source_block_id {
                                        if source_index < index {
                                            continue;
                                        }
                                    } else {
                                        if block_to_upstream_blocks[&block_id].contains(&source_block_id) {
                                            continue;
                                        }
                                    }
                                    result_builder.add_error(miette::MietteDiagnostic::new(format!("instruction {instruction:?} in {block_id:?} at index {index} uses {source_id:?} before assigned. Source defined in block {source_block_id:?} at index {source_index}")));
                                }
                            }
                        }
                    }
                }
            }
        }
        result_builder.finish(()).into_result()
    }

    pub fn remove_dangling_locals(&mut self) {
        for local in self.locals.keys().collect::<Vec<_>>() {
            if self.return_local == Some(local) {
                continue;
            }
            let dangling = self.locals[local]
                .defined
                .keys()
                .all(|defined_block| !self.blocks.contains_key(defined_block));
            if dangling {
                self.locals.remove(local);
            }
        }
    }

    pub fn check_local_integrity(&self) -> Result<(), MultiError> {
        Ok(())
    }
}

/// MIR Basic block
///
/// A straight-line sequence of instructions ending in a single [`Terminator`] that determines
/// which block (if any) executes next.
#[derive(Debug, Clone, Default)]
pub struct Block {
    /// The instructions executed in order when this block runs.
    pub instructions: Vec<Instr>,
    /// The instruction that determines how control leaves this block.
    pub terminator: Terminator,
    /// The blocks known to jump or call into this block.
    previous: IndexSet<BlockId>,
}

impl Block {
    /// Iterates the blocks this block's [Terminator] may transfer control to.
    pub fn next_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        let mut next = 0;
        std::iter::from_fn(move || {
            let block = match self.terminator {
                Terminator::Return => None,
                Terminator::Jump { target_block, .. }
                | Terminator::Call {
                    return_block: target_block,
                    ..
                }
                | Terminator::ReturnIf {
                    else_block: target_block,
                    ..
                } => match next {
                    0 => Some(target_block),
                    _ => None,
                },
                Terminator::JumpIf {
                    then_block,
                    else_block,
                    ..
                }
                | Terminator::CallIf {
                    return_block: then_block,
                    else_block,
                    ..
                } => match next {
                    0 => Some(then_block),
                    1 => Some(else_block),
                    _ => None,
                },
            }?;
            next += 1;
            Some(block)
        })
    }

    /// Iterates the blocks known to jump or call into this block.
    pub fn previous_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.previous.iter().copied()
    }
}

/// A global value: either a read-only constant (ROM) or a mutable global (RAM).
#[derive(Debug, Clone)]
pub struct Global {
    pub ty: Ty,
    pub name: SpannedIdentifier,
    pub attrs: GlobalAttributes,
    /// The global's constant value, encoded as bytes. Empty for mutable globals.
    pub value: Vec<u8>,
}

impl Global {
    /// Returns `true` if the global is a mutable (RAM) global rather than a read-only constant.
    #[must_use]
    pub fn is_mutable(&self) -> bool {
        match self.attrs {
            GlobalAttributes::Rom(_) => false,
            GlobalAttributes::Ram(_) => true,
        }
    }

    #[must_use]
    pub fn display<'a>(&'a self, ty_registry: &'a TyRegistry) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            match &self.attrs {
                GlobalAttributes::Rom(attrs) => {
                    writeln!(f, "@{:?}", attrs)?;
                    writeln!(
                        f,
                        "const {}: {} = {:?}",
                        self.name.data,
                        self.ty.display(ty_registry),
                        self.value
                    )?;
                }
                GlobalAttributes::Ram(attrs) => {
                    writeln!(f, "@{:?}", attrs)?;
                    writeln!(
                        f,
                        "global {}: {}",
                        self.name.data,
                        self.ty.display(ty_registry)
                    )?;
                }
            }
            Ok(())
        })
    }
}

/// The declaration kind and attributes of a [`Global`].
#[derive(Debug, Clone)]
pub enum GlobalAttributes {
    /// Attributes declared on the `const`.
    Rom(ConstDeclAttributes),
    /// Attributes declared on the `global`.
    Ram(GlobalDeclAttributes),
}

impl From<ConstDeclAttributes> for GlobalAttributes {
    fn from(attrs: ConstDeclAttributes) -> Self {
        GlobalAttributes::Rom(attrs)
    }
}

impl From<GlobalDeclAttributes> for GlobalAttributes {
    fn from(attrs: GlobalDeclAttributes) -> Self {
        GlobalAttributes::Ram(attrs)
    }
}

/// An SSA value (local variable) within a [`Function`].
///
/// Each local is defined by at most one instruction, per SSA form.
#[derive(Debug, Clone)]
pub struct Local {
    /// The type of the local's value.
    pub ty: Ty,
    /// Whether this local is the function's designated return-value local.
    ///
    /// Return locals are exempt from the "defined exactly once, before use" SSA rule enforced
    /// by [`FunctionBuilder::build`], since they are conceptually assigned once per exit path.
    pub is_return: bool,
    /// The block and instruction index that defines this local, if it has been defined yet.
    /// Only return locals should have more than one definition. All other locals will have
    /// one entry, or be empty for a dangling local.
    pub defined: SparseSecondaryMap<BlockId, usize>,
    /// The source-level variable name this local corresponds to, if any.
    pub name: Option<SpannedIdentifier>,
}

impl Local {
    #[must_use]
    pub fn display<'a>(&'a self, id: LocalId) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            if let Some(name) = self.name.as_ref() {
                write!(f, "{}", name.data)?;
            }
            f.write_str("#")?;
            if self.is_return {
                f.write_str("RET")?;
            } else {
                write!(f, "{}", slotmap::Key::data(&id).as_ffi() & 0xffff_ffff)?;
            }
            Ok(())
        })
    }

    #[must_use]
    pub fn display_with_ty<'a>(
        &'a self,
        id: LocalId,
        ty_registry: &'a TyRegistry,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| write!(f, "{}: {}", self.display(id), self.ty.display(ty_registry)))
    }
}

/// A MIR Instruction
///
/// Every variant assigns at most one [`LocalId`] (returned by [`Instr::target_local`]), except
/// [`Instr::Store`] which writes to a [`Global`] instead.
#[derive(Debug, Clone)]
pub enum Instr {
    // assignment
    /// Placeholder instruction which can be used to recover generating MIR due to an error.
    /// However, it is expected that build() is never called for MIR with blocks containing
    /// such instructions
    Undefined { target: LocalId },
    /// Allocates storage for a local without initializing it.
    Alloc { target: LocalId },
    /// Reads the current value of the function's return local.
    ReturnValue { target: LocalId },
    /// An SSA phi node: selects a value for `target` based on which predecessor block control
    /// arrived from, as recorded in `sources`.
    Phi {
        target: LocalId,
        sources: SparseSecondaryMap<BlockId, LocalId>,
    },
    /// Loads a value from a global (optionally projecting into a field or element of it) into
    /// `target`.
    Load {
        target: LocalId,
        source: GlobalId,
        projections: Vec<PlaceProjection>,
    },
    /// Stores `source` into a global (optionally projecting into a field or element of it).
    Store {
        target: GlobalId,
        projections: Vec<PlaceProjection>,
        source: Operand,
    },
    /// Copies an operand's value into `target`.
    Copy { target: LocalId, source: Operand },
    /// Reads a value from `source` (optionally projecting into a field or element of it) into
    /// `target`.
    Read {
        target: LocalId,
        source: LocalId,
        projections: Vec<PlaceProjection>,
    },
    /// Writes `right` into `left` (optionally projecting into a field or element of it),
    /// producing the updated aggregate value in `target`.
    Write {
        target: LocalId,
        left: LocalId,
        projections: Vec<PlaceProjection>,
        right: Operand,
    },
    // unary
    /// `!source`
    Not { target: LocalId, source: LocalId },
    /// `-source`
    Neg { target: LocalId, source: LocalId },
    /// Bitwise rotation of `source` to the left.
    RotateLeft { target: LocalId, source: LocalId },
    /// Bitwise rotation of `source` to the right.
    RotateRight { target: LocalId, source: LocalId },
    /// Bitwise shift of `source` to the left.
    ShiftLeft { target: LocalId, source: LocalId },
    /// Bitwise shift of `source` to the right.
    ShiftRight { target: LocalId, source: LocalId },
    // comparison
    /// `left == right`
    Eq {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left != right`
    Ne {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left < right`
    Lt {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left <= right`
    Le {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left >= right`
    Ge {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left > right`
    Gt {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    // arithmetic
    /// Bitwise AND of `left` and `right`.
    BitAnd {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// Bitwise OR of `left` and `right`.
    BitOr {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// Bitwise XOR of `left` and `right`.
    BitXor {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left + right` wrapping any overflow
    Add {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
    /// `left - right` wrapping any underflow
    Sub {
        target: LocalId,
        left: LocalId,
        right: Operand,
    },
}

impl Instr {
    /// Returns the local this instruction assigns, or `None` for [`Instr::Store`] which writes to
    /// a global instead.
    #[must_use]
    pub fn target_local(&self) -> Option<LocalId> {
        match self {
            Instr::Undefined { target, .. }
            | Instr::Alloc { target, .. }
            | Instr::ReturnValue { target, .. }
            | Instr::Phi { target, .. }
            | Instr::Load { target, .. }
            | Instr::Read { target, .. }
            | Instr::Write { target, .. }
            | Instr::Copy { target, .. }
            | Instr::Not { target, .. }
            | Instr::Neg { target, .. }
            | Instr::Eq { target, .. }
            | Instr::Ne { target, .. }
            | Instr::Lt { target, .. }
            | Instr::Le { target, .. }
            | Instr::Ge { target, .. }
            | Instr::Gt { target, .. }
            | Instr::BitAnd { target, .. }
            | Instr::BitOr { target, .. }
            | Instr::BitXor { target, .. }
            | Instr::RotateLeft { target, .. }
            | Instr::RotateRight { target, .. }
            | Instr::ShiftLeft { target, .. }
            | Instr::ShiftRight { target, .. }
            | Instr::Add { target, .. }
            | Instr::Sub { target, .. } => Some(*target),
            Instr::Store { .. } => None,
        }
    }

    /// Iterates the locals this instruction reads from, including locals used as index
    /// operands within its [`PlaceProjection`]s.
    pub fn source_locals(&self) -> impl Iterator<Item = LocalId> + '_ {
        let mut next = 0;
        let mut projections_offset = 0;
        std::iter::from_fn(move || {
            let value = match self {
                Instr::Undefined { .. } | Instr::Alloc { .. } | Instr::ReturnValue { .. } => None,
                Instr::Copy { source, .. } => match (next, source) {
                    (0, Operand::Local(local)) => Some(local),
                    _ => None,
                },
                Instr::Not { source, .. }
                | Instr::Neg { source, .. }
                | Instr::RotateLeft { source, .. }
                | Instr::RotateRight { source, .. }
                | Instr::ShiftLeft { source, .. }
                | Instr::ShiftRight { source, .. } => match next {
                    0 => Some(source),
                    _ => None,
                },
                Instr::Eq { left, right, .. }
                | Instr::Ne { left, right, .. }
                | Instr::Lt { left, right, .. }
                | Instr::Le { left, right, .. }
                | Instr::Ge { left, right, .. }
                | Instr::Gt { left, right, .. }
                | Instr::BitAnd { left, right, .. }
                | Instr::BitOr { left, right, .. }
                | Instr::BitXor { left, right, .. }
                | Instr::Add { left, right, .. }
                | Instr::Sub { left, right, .. } => match next {
                    0 => Some(left),
                    1 => match right {
                        Operand::Local(local) => Some(local),
                        _ => None,
                    },
                    _ => None,
                },
                Instr::Phi { sources, .. } => sources.values().nth(next),
                this @ Instr::Load { .. }
                | this @ Instr::Store { .. }
                | this @ Instr::Read { .. }
                | this @ Instr::Write { .. } => match (next, this) {
                    (
                        0,
                        Instr::Store {
                            source: Operand::Local(local),
                            ..
                        },
                    ) => Some(local),
                    (0, Instr::Read { source, .. }) => Some(source),
                    (0, Instr::Write { left, .. }) => Some(left),
                    (
                        1,
                        Instr::Write {
                            right: Operand::Local(local),
                            ..
                        },
                    ) => Some(local),
                    (
                        _,
                        Instr::Load { projections, .. }
                        | Instr::Store { projections, .. }
                        | Instr::Read { projections, .. }
                        | Instr::Write { projections, .. },
                    ) => {
                        let (index, local) = projections
                            .iter()
                            .enumerate()
                            .skip(projections_offset)
                            .find_map(|(i, projection)| {
                                if let PlaceProjection::Index(Operand::Local(local)) = projection {
                                    Some((i, local))
                                } else {
                                    None
                                }
                            })?;
                        projections_offset = index + 1;
                        Some(local)
                    }
                    _ => unreachable!(
                        "Only load, store, read and write instructions reach this match"
                    ),
                },
            }?;
            next += 1;
            Some(*value)
        })
    }
}

/// The instruction that ends a [`Block`], determining which block(s) execution may transfer to
/// next.
#[derive(Debug, Clone, Default)]
pub enum Terminator {
    /// Returns from the function.
    #[default]
    Return,
    /// Unconditionally calls `callee` and, once it returns, transfers control to
    /// `return_block`.
    Call {
        callee: Callee,
        return_block: BlockId,
    },
    /// Tests `source` against `condition`: if true, calls `callee` and transfers control to
    /// `return_block` once it returns; otherwise transfers control directly to `else_block`.
    CallIf {
        source: LocalId,
        condition: Condition,
        callee: Callee,
        return_block: BlockId,
        else_block: BlockId,
    },
    /// Unconditionally transfers control to `target_block`.
    Jump { target_block: BlockId },
    /// Tests `source` against `condition` and transfers control to `then_block` if true, or
    /// `else_block` otherwise.
    JumpIf {
        source: LocalId,
        condition: Condition,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Tests `source` against `condition`: if true, returns from the function; otherwise
    /// transfers control to `else_block`.
    ReturnIf {
        source: LocalId,
        condition: Condition,
        else_block: BlockId,
    },
}

/// The target of a [`Terminator::Call`] or [`Terminator::CallIf`].
#[derive(Debug, Clone)]
pub enum Callee {
    /// Calls a function pointer held in a local.
    Local(LocalId),
    /// Calls a statically known function.
    FnPtr(FunctionId),
}

impl Callee {
    #[must_use]
    pub fn display<'a>(&'a self, mir: &'a Mir, function: &'a Function) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Callee::Local(local) => write!(f, "{}", local.display(function)),
            Callee::FnPtr(fn_ptr) => write!(f, "{}", mir.functions[*fn_ptr].name),
        })
    }
}

/// A value used as the operand of an [`Instr`]: either read from a local, given as a literal, or
/// referencing a function pointer.
#[derive(Debug, Clone)]
pub enum Operand {
    /// The value of a local.
    Local(LocalId),
    /// A constant integer literal.
    Literal(IntegerLiteral),
    /// A pointer to a statically known function.
    FnPtr(FunctionId),
}

impl Operand {
    #[must_use]
    pub fn display<'a>(&'a self, mir: &'a Mir, function: &'a Function) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| match self {
            Operand::Local(local) => write!(f, "{}", local.display(function)),
            Operand::Literal(int) => write!(f, "{}", int.value),
            Operand::FnPtr(fn_ptr) => write!(f, "{}", mir.functions[*fn_ptr].name),
        })
    }
}

/// A single step of a place projection, used by [`Instr::Load`], [`Instr::Store`], [`Instr::Read`]
/// and [`Instr::Write`] to navigate into a field or element of an aggregate value.
#[derive(Debug, Clone)]
pub enum PlaceProjection {
    /// Projects into a named field, e.g. `.field`.
    Field(Identifier),
    /// Projects into an indexed element, e.g. `[index]`.
    Index(Operand),
}

impl PlaceProjection {
    #[must_use]
    pub fn display<'a>(&'a self, mir: &'a Mir, function: &'a Function) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            match self {
                PlaceProjection::Field(field) => write!(f, ".{field}"),
                PlaceProjection::Index(index) => {
                    write!(f, "[{}]", index.display(mir, function))
                }
            }?;
            Ok(())
        })
    }
}

/// The condition tested by a conditional [`Terminator`] against its `source` local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// True when `source` is zero.
    Zero,
    /// True when `source` is non-zero.
    NotZero,
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Condition::Zero => "== 0",
            Condition::NotZero => "!= 0",
        })
    }
}

impl Not for Condition {
    type Output = Condition;

    fn not(self) -> Self::Output {
        match self {
            Condition::Zero => Condition::NotZero,
            Condition::NotZero => Condition::Zero,
        }
    }
}

/// A source-level variable used during SSA construction (via the Braun et al.'s algorithm)
/// by [`FunctionBuilder`].
///
/// Unlike [`LocalId`], a `Variable` may be written to and read from multiple times across a
/// function's blocks; [`FunctionBuilder::write_variable`] and [`FunctionBuilder::read_variable`]
/// resolve each read to the appropriate SSA [`LocalId`], inserting [`Instr::Phi`] nodes as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variable(u32);

/// Incrementally builds a [Mir].
#[derive(Default)]
pub struct MirBuilder {
    mir: Mir,
    sealed_functions: SecondaryMap<FunctionId, FunctionBuilder>,
}

impl MirBuilder {
    pub fn insert_global(
        &mut self,
        name: SpannedIdentifier,
        ty: Ty,
        attrs: GlobalAttributes,
    ) -> GlobalId {
        self.mir.globals.insert(Global {
            name,
            ty,
            attrs,
            value: Vec::new(),
        })
    }

    #[must_use]
    pub fn is_mutable_global(&self, id: GlobalId) -> bool {
        self.mir.globals[id].is_mutable()
    }

    #[must_use]
    pub fn global_name(&self, id: GlobalId) -> &SpannedIdentifier {
        &self.mir.globals[id].name
    }

    #[must_use]
    pub fn global_value(&self, id: GlobalId) -> &[u8] {
        self.mir.globals[id].value.as_slice()
    }

    #[must_use]
    pub fn global_ty(&self, id: GlobalId) -> Ty {
        self.mir.globals[id].ty
    }

    pub fn set_global_value(&mut self, id: GlobalId, value: Vec<u8>) {
        let global = &mut self.mir.globals[id];
        // mutable is reflecting runtime mutability, which is the inverse
        // at compile time (as you need to define values for all constants
        // at compile time
        assert!(!global.is_mutable(), "can only set value for constants");
        assert!(
            global.value.is_empty(),
            "value already set for constant {id:?}"
        );
        assert!(!value.is_empty(), "value must not be empty");
        global.value = value;
    }

    pub fn reserve_function(&mut self) -> FunctionId {
        self.mir.functions.insert(Function::default())
    }

    pub fn seal_function(&mut self, function_id: FunctionId, builder: FunctionBuilder) {
        let existing = self.sealed_functions.insert(function_id, builder);
        assert!(
            existing.is_none(),
            "Function {function_id:?} already sealed"
        );
    }

    #[must_use]
    pub fn build(mut self) -> Mir {
        let functions = self.mir.functions.keys().collect::<Vec<_>>();
        for function in functions {
            match self.sealed_functions.remove(function) {
                None => {
                    self.mir.functions.remove(function);
                }
                Some(builder) => {
                    self.mir.functions[function] = builder.build();
                }
            }
        }
        self.mir
    }
}

/// Incrementally builds a [`Function`], performing SSA construction (including phi node
/// insertion and trivial phi elimination) as blocks are sealed.
pub struct FunctionBuilder {
    function: Function,
    variable_assignments: IndexMap<Variable, IndexMap<BlockId, Vec<LocalId>>>,
    variable_names: HashMap<Variable, SpannedIdentifier>,
    terminated_blocks: SecondaryMap<BlockId, ()>,
    sealed_blocks: SecondaryMap<BlockId, ()>,
    incomplete_phis: HashMap<BlockId, IndexMap<Variable, usize>>,
    next_var: u32,
}

impl FunctionBuilder {
    #[must_use]
    pub fn new(name: Identifier, return_ty: Ty) -> Self {
        FunctionBuilder {
            function: Function {
                name,
                return_ty,
                ..Default::default()
            },
            variable_assignments: IndexMap::new(),
            variable_names: HashMap::new(),
            terminated_blocks: SecondaryMap::new(),
            sealed_blocks: SecondaryMap::new(),
            incomplete_phis: HashMap::new(),
            next_var: 0,
        }
    }

    pub fn set_attrs(&mut self, attrs: FnDeclAttributes) {
        self.function.attrs = attrs;
    }

    #[must_use]
    pub fn build(mut self) -> Function {
        self.function.remove_dangling();
        if cfg!(debug_assertions) {
            for block_id in self.function.blocks.keys() {
                assert!(
                    self.sealed_blocks.contains_key(block_id),
                    "{block_id} must be sealed"
                );
            }
        }
        #[cfg(debug_assertions)]
        self.function.check_integrity().unwrap();
        self.function
    }
}

impl FunctionBuilder {
    pub fn new_variable(&mut self) -> Variable {
        let var = Variable(self.next_var);
        self.next_var += 1;
        var
    }

    pub fn set_variable_name(&mut self, variable: Variable, name: SpannedIdentifier) {
        let previous = self.variable_names.insert(variable, name);
        debug_assert!(
            previous.is_none(),
            "{variable:?} was already named {previous:?}"
        );
    }

    pub fn new_block(&mut self) -> BlockId {
        let block = self.function.blocks.insert(Block::default());
        if self.function.blocks.len() == 1 {
            self.function.start_block = block;
        }
        block
    }

    pub fn new_local(&mut self, ty: Ty) -> LocalId {
        self.function.locals.insert(Local {
            ty,
            is_return: false,
            defined: SparseSecondaryMap::new(),
            name: None,
        })
    }

    pub fn set_local_ty(&mut self, local: LocalId, ty: Ty) {
        self.function.locals[local].ty = ty;
    }

    pub fn propagate_variable_types(&mut self) {
        for assignments in self.variable_assignments.values() {
            let ty = assignments.values().flatten().find_map(|&local| {
                let ty = self.function.locals[local].ty;
                if ty == Ty::UNRESOLVED { None } else { Some(ty) }
            });
            if let Some(ty) = ty {
                for &local in assignments.values().flatten() {
                    if self.function.locals[local].ty == Ty::UNRESOLVED {
                        self.function.locals[local].ty = ty;
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn is_named_local(&self, local: LocalId) -> bool {
        self.function.locals[local].name.is_some()
    }

    pub fn create_return_local(&mut self, ty: Ty) -> LocalId {
        assert!(self.function.return_local.is_some() || self.function.locals.is_empty());
        if self.function.return_local.is_none() {
            let local = self.function.locals.insert(Local {
                ty,
                is_return: true,
                defined: SparseSecondaryMap::new(),
                name: None,
            });
            self.function.return_local = Some(local);
        }
        self.function.return_local.unwrap()
    }

    pub fn push_instr(&mut self, block: BlockId, instr: Instr) -> usize {
        let target = instr.target_local();
        let instructions = &mut self.function.blocks[block].instructions;
        let index = instructions.len();
        instructions.push(instr);
        if let Some(target) = target {
            assert!(
                self.function.locals[target].defined.is_empty()
                    || self.function.return_local == Some(target),
                "local {target:?} defined previously {:?}",
                self.function.locals[target].defined
            );
            match self.function.locals[target]
                .defined
                .entry(block)
                .expect("block already valid above")
            {
                Entry::Occupied(entry) => {
                    panic!(
                        "local {target:?} already defined in block {block:?} at index {}. Cannot refine to index {index}",
                        entry.get()
                    )
                }
                Entry::Vacant(entry) => {
                    entry.insert(index);
                }
            }
        }
        index
    }

    pub fn set_terminator(&mut self, block: BlockId, terminator: Terminator) {
        debug_assert!(
            !self.terminated_blocks.contains_key(block),
            "block {block:?} is already terminated"
        );
        match &terminator {
            Terminator::Return => {
                self.function.end_blocks.insert(block);
            }
            Terminator::ReturnIf {
                else_block: target_block,
                ..
            } => {
                self.function.blocks[*target_block].previous.insert(block);
                self.function.end_blocks.insert(block);
            }
            Terminator::Call {
                return_block: target_block,
                ..
            }
            | Terminator::Jump { target_block, .. } => {
                self.function.blocks[*target_block].previous.insert(block);
            }
            Terminator::CallIf {
                return_block: then_block,
                else_block,
                ..
            }
            | Terminator::JumpIf {
                then_block,
                else_block,
                ..
            } => {
                self.function.blocks[*then_block].previous.insert(block);
                self.function.blocks[*else_block].previous.insert(block);
            }
        }
        self.function.blocks[block].terminator = terminator;
        self.terminated_blocks.insert(block, ());
    }

    /// Record the `local` as the current value for `var` in `block` (as per `writeVariable`
    /// defined in the Braun et al.'s algorithm)
    pub fn write_variable(&mut self, var: Variable, block: BlockId, local: LocalId) {
        self.variable_assignments
            .entry(var)
            .or_default()
            .entry(block)
            .or_default()
            .push(local);
        if let Some(name) = self.variable_names.get(&var)
            && self.function.locals[local].name.is_none()
        {
            self.function.locals[local].name = Some(name.clone());
        }
    }

    /// Read the effective `local` for the `var` in `block` (as per `readVariable`
    /// defined in the Braun et al.'s algorithm). If the block does not assign
    /// `var`, `read_variable_recursive` will be used to resolve the effective local.
    pub fn read_variable(&mut self, var: Variable, block: BlockId) -> LocalId {
        if let Some(value) = self
            .variable_assignments
            .get(&var)
            .and_then(|assignments| assignments.get(&block))
            .and_then(|assignments| assignments.last())
        {
            return *value;
        }
        self.read_variable_recursive(var, block)
    }

    /// Read the effective `local` for the `var` in `block` (as per `readVariableRecursive`
    /// defined in the Braun et al.'s algorithm) by traversing previous blocks, inserting
    /// phi nodes when necessary.
    fn read_variable_recursive(&mut self, var: Variable, block: BlockId) -> LocalId {
        let value = if !self.sealed_blocks.contains_key(block) {
            let target = self.new_local(Ty::UNRESOLVED);
            let index = self.push_instr(
                block,
                Instr::Phi {
                    target,
                    sources: SparseSecondaryMap::new(),
                },
            );
            self.incomplete_phis
                .entry(block)
                .or_default()
                .insert(var, index);
            target
        } else {
            let mut previous_blocks = self.function.blocks[block].previous.iter();
            match (previous_blocks.next(), previous_blocks.next()) {
                (None, None) => {
                    panic!("attempted to read variable {var:?} from dangling block {block:?}")
                }
                (Some(single), None) => self.read_variable(var, *single),
                _ => {
                    let target = self.new_local(Ty::UNRESOLVED);
                    let index = self.push_instr(
                        block,
                        Instr::Phi {
                            target,
                            sources: SparseSecondaryMap::new(),
                        },
                    );
                    self.write_variable(var, block, target);
                    self.add_phi_sources(var, block, target, index)
                }
            }
        };
        self.write_variable(var, block, value);
        value
    }

    fn add_phi_sources(
        &mut self,
        var: Variable,
        block: BlockId,
        phi: LocalId,
        index: usize,
    ) -> LocalId {
        // need to create a clone as self.read_variable requires &mut self
        let previous_blocks = self.function.blocks[block]
            .previous
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for previous in previous_blocks {
            let value = self.read_variable(var, previous);
            match &mut self.function.blocks[block].instructions[index] {
                Instr::Phi { sources, .. } => {
                    sources.insert(previous, value);
                }
                _ => {
                    unreachable!("phi placeholder was overwritten before its operands were filled")
                }
            }
        }
        let resolved = self.try_remove_trivial_phi(block, phi, index);
        self.write_variable(var, block, resolved);
        resolved
    }

    fn try_remove_trivial_phi(&mut self, block: BlockId, target: LocalId, index: usize) -> LocalId {
        let sources = match &self.function.blocks[block].instructions[index] {
            Instr::Phi { sources, .. } => sources.values().copied().collect::<Vec<_>>(),
            inst => {
                return inst
                    .target_local()
                    .expect("must be an instruction which targets a ");
            }
        };
        // trivial phis are defined as a phi which references a single other source other than
        // itself. sources may contain itself due to a cycle in blocks
        let mut single_source: Option<LocalId> = None;
        for source in sources {
            let source = self.resolve(source);
            if source == target {
                continue;
            }
            match single_source {
                Some(existing) if existing != source => return target,
                _ => single_source = Some(source),
            }
        }
        match single_source {
            Some(source) => {
                self.function.blocks[block].instructions[index] = Instr::Copy {
                    target,
                    source: Operand::Local(source),
                };
                source
            }
            None => target,
        }
    }

    fn resolve(&self, mut local: LocalId) -> LocalId {
        while self.function.locals[local].defined.len() == 1 {
            let (block, &index) = self.function.locals[local].defined.iter().next().unwrap();
            if let Instr::Copy {
                source: Operand::Local(source),
                ..
            } = self.function.blocks[block].instructions[index]
            {
                local = source;
            } else {
                break;
            }
        }
        local
    }

    pub fn seal_block(&mut self, block: BlockId) {
        if let Some(pending) = self.incomplete_phis.remove(&block) {
            for (var, index) in pending {
                let phi = self.function.blocks[block].instructions[index]
                    .target_local()
                    .expect("target");
                self.add_phi_sources(var, block, phi, index);
            }
        }
        self.sealed_blocks.insert(block, ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SpannedIdentifier;
    use miette::{SourceOffset, SourceSpan};

    fn dummy_span() -> SourceSpan {
        SourceSpan::new(SourceOffset::from(0), 0)
    }

    fn ident(name: &str) -> SpannedIdentifier {
        SpannedIdentifier::new(name, dummy_span())
    }

    fn new_block_ids(n: usize) -> (SlotMap<BlockId, Block>, Vec<BlockId>) {
        let mut blocks = SlotMap::with_key();
        let ids = (0..n).map(|_| blocks.insert(Block::default())).collect();
        (blocks, ids)
    }

    fn new_local_ids(n: usize) -> (SlotMap<LocalId, Local>, Vec<LocalId>) {
        let mut locals = SlotMap::with_key();
        let ids = (0..n)
            .map(|_| {
                locals.insert(Local {
                    ty: Ty::U8,
                    is_return: false,
                    defined: SparseSecondaryMap::new(),
                    name: None,
                })
            })
            .collect();
        (locals, ids)
    }

    fn make_function(blocks: SlotMap<BlockId, Block>, start: BlockId) -> Function {
        Function {
            name: Identifier::from("f"),
            attrs: FnDeclAttributes::default(),
            return_ty: Ty::UNIT,
            return_local: None,
            blocks,
            locals: SlotMap::with_key(),
            start_block: start,
            end_blocks: IndexSet::new(),
        }
    }

    fn new_builder() -> FunctionBuilder {
        FunctionBuilder::new(Identifier::from("f"), Ty::UNIT)
    }

    fn trivially_built_function() -> Function {
        let mut fb = new_builder();
        let block = fb.new_block();
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        fb.build()
    }

    // --- Block::next_blocks / previous_blocks ---

    #[test]
    fn next_blocks_return_is_empty() {
        let block = Block {
            instructions: vec![],
            terminator: Terminator::Return,
            previous: IndexSet::new(),
        };
        assert_eq!(block.next_blocks().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    fn next_blocks_jump_yields_target() {
        let (_blocks, ids) = new_block_ids(1);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::Jump {
                target_block: ids[0],
            },
            previous: IndexSet::new(),
        };
        assert_eq!(block.next_blocks().collect::<Vec<_>>(), vec![ids[0]]);
    }

    #[test]
    fn next_blocks_call_yields_return_block() {
        let (_blocks, ids) = new_block_ids(2);
        let (_locals, local_ids) = new_local_ids(1);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::Call {
                callee: Callee::Local(local_ids[0]),
                return_block: ids[0],
            },
            previous: IndexSet::new(),
        };
        assert_eq!(block.next_blocks().collect::<Vec<_>>(), vec![ids[0]]);
        let _ = ids[1];
    }

    #[test]
    fn next_blocks_return_if_yields_only_else_block() {
        let (_blocks, ids) = new_block_ids(1);
        let (_locals, local_ids) = new_local_ids(1);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::ReturnIf {
                source: local_ids[0],
                condition: Condition::Zero,
                else_block: ids[0],
            },
            previous: IndexSet::new(),
        };
        assert_eq!(block.next_blocks().collect::<Vec<_>>(), vec![ids[0]]);
    }

    #[test]
    fn next_blocks_jump_if_yields_then_and_else() {
        let (_blocks, ids) = new_block_ids(2);
        let (_locals, local_ids) = new_local_ids(1);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::JumpIf {
                source: local_ids[0],
                condition: Condition::NotZero,
                then_block: ids[0],
                else_block: ids[1],
            },
            previous: IndexSet::new(),
        };
        assert_eq!(
            block.next_blocks().collect::<Vec<_>>(),
            vec![ids[0], ids[1]]
        );
    }

    #[test]
    fn next_blocks_call_if_yields_return_and_else() {
        let (_blocks, ids) = new_block_ids(2);
        let (_locals, local_ids) = new_local_ids(2);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::CallIf {
                source: local_ids[0],
                condition: Condition::NotZero,
                callee: Callee::Local(local_ids[1]),
                return_block: ids[0],
                else_block: ids[1],
            },
            previous: IndexSet::new(),
        };
        assert_eq!(
            block.next_blocks().collect::<Vec<_>>(),
            vec![ids[0], ids[1]]
        );
    }

    #[test]
    fn previous_blocks_returns_recorded_predecessors() {
        let (_blocks, ids) = new_block_ids(2);
        let block = Block {
            instructions: vec![],
            terminator: Terminator::Return,
            previous: [ids[0], ids[1]].into_iter().collect(),
        };
        assert_eq!(
            block.previous_blocks().collect::<Vec<_>>(),
            vec![ids[0], ids[1]]
        );
    }

    // --- Function::traverse_blocks ---

    #[test]
    fn traverse_blocks_visits_diamond_in_bfs_order_without_duplicates() {
        let (mut blocks, ids) = new_block_ids(4);
        let (_locals, local_ids) = new_local_ids(1);
        blocks[ids[0]].terminator = Terminator::JumpIf {
            source: local_ids[0],
            condition: Condition::NotZero,
            then_block: ids[1],
            else_block: ids[2],
        };
        blocks[ids[1]].terminator = Terminator::Jump {
            target_block: ids[3],
        };
        blocks[ids[2]].terminator = Terminator::Jump {
            target_block: ids[3],
        };
        blocks[ids[3]].terminator = Terminator::Return;

        let function = make_function(blocks, ids[0]);
        let order = function.traverse_blocks().collect::<Vec<_>>();
        assert_eq!(order, vec![ids[0], ids[1], ids[2], ids[3]]);
    }

    #[test]
    fn traverse_blocks_terminates_and_deduplicates_on_a_cycle() {
        let (mut blocks, ids) = new_block_ids(3);
        let (_locals, local_ids) = new_local_ids(1);
        blocks[ids[0]].terminator = Terminator::JumpIf {
            source: local_ids[0],
            condition: Condition::NotZero,
            then_block: ids[1],
            else_block: ids[2],
        };
        blocks[ids[1]].terminator = Terminator::Jump {
            target_block: ids[0],
        };
        blocks[ids[2]].terminator = Terminator::Return;

        let function = make_function(blocks, ids[0]);
        let order = function.traverse_blocks().collect::<Vec<_>>();
        assert_eq!(order, vec![ids[0], ids[1], ids[2]]);
    }

    // --- Instr::target_local / source_locals ---

    #[test]
    fn target_local_variants() {
        let (_locals, ids) = new_local_ids(2);
        let mut globals: SlotMap<GlobalId, Global> = SlotMap::with_key();
        let global = globals.insert(Global {
            ty: Ty::U8,
            name: ident("g"),
            attrs: GlobalDeclAttributes::default().into(),
            value: Vec::new(),
        });

        assert_eq!(
            Instr::Undefined { target: ids[0] }.target_local(),
            Some(ids[0])
        );
        assert_eq!(
            Instr::ReturnValue { target: ids[0] }.target_local(),
            Some(ids[0])
        );
        assert_eq!(
            Instr::Copy {
                target: ids[0],
                source: Operand::Local(ids[1])
            }
            .target_local(),
            Some(ids[0])
        );
        assert_eq!(
            Instr::Store {
                target: global,
                projections: vec![],
                source: Operand::Local(ids[1])
            }
            .target_local(),
            None
        );
    }

    #[test]
    fn source_locals_for_binary_instr_lists_both_locals() {
        let (_locals, ids) = new_local_ids(3);
        let instr = Instr::Add {
            target: ids[0],
            left: ids[1],
            right: Operand::Local(ids[2]),
        };
        assert_eq!(
            instr.source_locals().collect::<Vec<_>>(),
            vec![ids[1], ids[2]]
        );
    }

    #[test]
    fn source_locals_skips_non_local_operands() {
        let (_locals, ids) = new_local_ids(2);
        let instr = Instr::Add {
            target: ids[0],
            left: ids[1],
            right: Operand::Literal(IntegerLiteral { value: 5 }),
        };
        assert_eq!(instr.source_locals().collect::<Vec<_>>(), vec![ids[1]]);
    }

    #[test]
    fn source_locals_for_load_and_undefined_are_empty() {
        let (_locals, ids) = new_local_ids(1);
        let mut globals: SlotMap<GlobalId, Global> = SlotMap::with_key();
        let global = globals.insert(Global {
            ty: Ty::U8,
            name: ident("g"),
            attrs: GlobalDeclAttributes::default().into(),
            value: Vec::new(),
        });
        let load = Instr::Load {
            target: ids[0],
            source: global,
            projections: vec![],
        };
        assert_eq!(load.source_locals().collect::<Vec<_>>(), vec![]);
        assert_eq!(
            Instr::Undefined { target: ids[0] }
                .source_locals()
                .collect::<Vec<_>>(),
            vec![]
        );
    }

    #[test]
    fn source_locals_for_phi_lists_all_source_locals() {
        let (_locals, ids) = new_local_ids(3);
        let (_blocks, block_ids) = new_block_ids(2);
        let mut sources = SparseSecondaryMap::new();
        sources.insert(block_ids[0], ids[1]);
        sources.insert(block_ids[1], ids[2]);
        let instr = Instr::Phi {
            target: ids[0],
            sources,
        };
        let result = instr.source_locals().collect::<Vec<_>>();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&ids[1]));
        assert!(result.contains(&ids[2]));
    }

    // --- FunctionBuilder basics ---

    #[test]
    fn new_block_first_call_sets_start_block() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let _b1 = fb.new_block();
        assert_eq!(fb.function.start_block, b0);
    }

    #[test]
    fn create_return_local_is_idempotent() {
        let mut fb = new_builder();
        let a = fb.create_return_local(Ty::U8);
        let b = fb.create_return_local(Ty::U8);
        assert_eq!(a, b);
        assert_eq!(fb.function.locals.len(), 1);
        assert!(fb.function.locals[a].is_return);
    }

    #[test]
    fn push_instr_records_local_definition_site() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let local = fb.new_local(Ty::U8);
        let index = fb.push_instr(
            block,
            Instr::Copy {
                target: local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        assert_eq!(index, 0);
        assert_eq!(fb.function.locals[local].defined.get(block), Some(&0));
    }

    #[test]
    fn push_instr_does_records_definition_for_return_local() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret_local = fb.create_return_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret_local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        assert_eq!(fb.function.locals[ret_local].defined.get(block), Some(&0));
    }

    #[test]
    fn push_instr_does_records_definition_for_return_local_when_divergent_blocks() {
        let mut fb = new_builder();
        let block1 = fb.new_block();
        let block2 = fb.new_block();
        let ret_local = fb.create_return_local(Ty::U8);
        fb.push_instr(
            block1,
            Instr::Copy {
                target: ret_local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        fb.push_instr(
            block2,
            Instr::Copy {
                target: ret_local,
                source: Operand::Literal(IntegerLiteral { value: 2 }),
            },
        );
        assert_eq!(fb.function.locals[ret_local].defined.get(block1), Some(&0));
        assert_eq!(fb.function.locals[ret_local].defined.get(block2), Some(&0));
    }

    #[test]
    #[should_panic(expected = "already defined in block")]
    fn panics_when_return_local_assigned_twice_in_same_block() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let ret_local = fb.create_return_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret_local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        fb.push_instr(
            block,
            Instr::Copy {
                target: ret_local,
                source: Operand::Literal(IntegerLiteral { value: 2 }),
            },
        );
    }

    #[test]
    #[should_panic(expected = "defined previously")]
    fn push_instr_panics_when_local_assigned_twice() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let local = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        fb.push_instr(
            block,
            Instr::Copy {
                target: local,
                source: Operand::Literal(IntegerLiteral { value: 2 }),
            },
        );
    }

    #[test]
    fn set_terminator_jump_records_predecessor() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let b1 = fb.new_block();
        fb.set_terminator(b0, Terminator::Jump { target_block: b1 });
        assert_eq!(
            fb.function.blocks[b1].previous_blocks().collect::<Vec<_>>(),
            vec![b0]
        );
        assert!(fb.function.end_blocks.is_empty());
    }

    #[test]
    fn set_terminator_return_adds_to_end_blocks() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        fb.set_terminator(b0, Terminator::Return);
        assert_eq!(
            fb.function.end_blocks.iter().copied().collect::<Vec<_>>(),
            vec![b0]
        );
    }

    #[test]
    fn set_terminator_jump_if_records_both_predecessors() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let cond_local = fb.new_local(Ty::U1);
        fb.set_terminator(
            b0,
            Terminator::JumpIf {
                source: cond_local,
                condition: Condition::NotZero,
                then_block: b1,
                else_block: b2,
            },
        );
        assert_eq!(
            fb.function.blocks[b1].previous_blocks().collect::<Vec<_>>(),
            vec![b0]
        );
        assert_eq!(
            fb.function.blocks[b2].previous_blocks().collect::<Vec<_>>(),
            vec![b0]
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "already terminated")]
    fn set_terminator_twice_panics() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        fb.set_terminator(b0, Terminator::Return);
        fb.set_terminator(b0, Terminator::Return);
    }

    // --- SSA variable renaming (write_variable / read_variable) ---

    #[test]
    fn write_then_read_in_same_block_returns_written_local() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let var = fb.new_variable();
        let local = fb.new_local(Ty::U8);
        fb.write_variable(var, block, local);
        assert_eq!(fb.read_variable(var, block), local);
    }

    #[test]
    fn read_variable_propagates_through_single_sealed_predecessor_without_phi() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let b1 = fb.new_block();
        let var = fb.new_variable();
        let local = fb.new_local(Ty::U8);
        fb.write_variable(var, b0, local);
        fb.set_terminator(b0, Terminator::Jump { target_block: b1 });
        fb.seal_block(b0);
        fb.seal_block(b1);

        assert_eq!(fb.read_variable(var, b1), local);
        assert!(fb.function.blocks[b1].instructions.is_empty());
    }

    #[test]
    fn read_variable_at_diamond_merge_creates_non_trivial_phi() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let b1 = fb.new_block();
        let b2 = fb.new_block();
        let b3 = fb.new_block();

        let var = fb.new_variable();
        let cond_local = fb.new_local(Ty::U1);
        fb.set_terminator(
            b0,
            Terminator::JumpIf {
                source: cond_local,
                condition: Condition::NotZero,
                then_block: b1,
                else_block: b2,
            },
        );
        fb.seal_block(b0);

        let local1 = fb.new_local(Ty::U8);
        fb.write_variable(var, b1, local1);
        fb.set_terminator(b1, Terminator::Jump { target_block: b3 });
        fb.seal_block(b1);

        let local2 = fb.new_local(Ty::U8);
        fb.write_variable(var, b2, local2);
        fb.set_terminator(b2, Terminator::Jump { target_block: b3 });
        fb.seal_block(b2);

        fb.seal_block(b3);

        let merged = fb.read_variable(var, b3);
        assert_ne!(merged, local1);
        assert_ne!(merged, local2);
        match &fb.function.blocks[b3].instructions[0] {
            Instr::Phi { target, sources } => {
                assert_eq!(*target, merged);
                let values = sources.values().copied().collect::<Vec<_>>();
                assert_eq!(values.len(), 2);
                assert!(values.contains(&local1));
                assert!(values.contains(&local2));
            }
            other => panic!("expected a Phi instruction, got {other:?}"),
        }
    }

    #[test]
    fn loop_carried_variable_unchanged_in_body_collapses_to_trivial_phi() {
        // b0 (writes local0) -> b1 (loop header, reads var before sealed) -> b2 (loop body, back edge to b1)
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let var = fb.new_variable();
        let local0 = fb.new_local(Ty::U8);
        fb.write_variable(var, b0, local0);

        let b1 = fb.new_block();
        fb.set_terminator(b0, Terminator::Jump { target_block: b1 });
        fb.seal_block(b0);

        // b1 is not sealed yet: this must create a placeholder Phi.
        let read_before_seal = fb.read_variable(var, b1);
        assert_eq!(fb.function.blocks[b1].instructions.len(), 1);
        assert!(matches!(
            fb.function.blocks[b1].instructions[0],
            Instr::Phi { .. }
        ));

        let b2 = fb.new_block();
        fb.set_terminator(b1, Terminator::Jump { target_block: b2 });
        fb.set_terminator(b2, Terminator::Jump { target_block: b1 });
        fb.seal_block(b2);
        // Sealing b1 now resolves the placeholder using its two predecessors (b0, b2). Since
        // the loop body never rewrote `var`, both predecessors resolve to local0, making the
        // phi trivial and collapsing it into a Copy.
        fb.seal_block(b1);

        assert_eq!(fb.read_variable(var, b1), local0);
        assert_ne!(read_before_seal, local0);
        match &fb.function.blocks[b1].instructions[0] {
            Instr::Copy { target, source } => {
                assert_eq!(*target, read_before_seal);
                assert!(matches!(source, Operand::Local(l) if *l == local0));
            }
            other => panic!("expected trivial phi to collapse into a Copy, got {other:?}"),
        }
    }

    // --- FunctionBuilder::build() ---

    #[test]
    fn build_removes_unreachable_blocks_and_unused_locals() {
        let mut fb = new_builder();
        let b0 = fb.new_block();
        let dangling_block = fb.new_block();
        let dangling_local = fb.new_local(Ty::U8);

        fb.set_terminator(b0, Terminator::Return);
        fb.seal_block(b0);

        let function = fb.build();
        assert!(!function.blocks.contains_key(dangling_block));
        assert!(!function.locals.contains_key(dangling_local));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "must be sealed")]
    fn build_panics_when_reachable_block_not_sealed() {
        let mut fb = new_builder();
        let block = fb.new_block();
        fb.set_terminator(block, Terminator::Return);
        let _ = fb.build();
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "Undefined instruction")]
    fn build_panics_on_leftover_undefined_instruction() {
        let mut fb = new_builder();
        let block = fb.new_block();
        let local = fb.new_local(Ty::U8);
        fb.push_instr(block, Instr::Undefined { target: local });
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let _ = fb.build();
    }

    // --- MirBuilder ---

    #[test]
    fn mir_builder_insert_global_creates_global() {
        let mut builder = MirBuilder::default();
        let name = ident("g");
        let id =
            builder.insert_global(name.clone(), Ty::U8, GlobalDeclAttributes::default().into());
        let mir = builder.build();
        assert_eq!(mir.globals[id].ty, Ty::U8);
        assert_eq!(mir.globals[id].name.data, name.data);
    }

    #[test]
    fn mir_builder_build_removes_unsealed_functions() {
        let mut builder = MirBuilder::default();
        let fn_id = builder.reserve_function();
        let mir = builder.build();
        assert!(!mir.functions.contains_key(fn_id));
    }

    #[test]
    fn mir_builder_build_keeps_sealed_functions() {
        let mut builder = MirBuilder::default();
        let fn_id = builder.reserve_function();
        let mut fb = new_builder();
        let block = fb.new_block();
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        builder.seal_function(fn_id, fb);

        let mir = builder.build();
        assert_eq!(mir.functions[fn_id].name, Identifier::from("f"));
    }

    #[test]
    #[should_panic(expected = "already sealed")]
    fn mir_builder_seal_function_twice_panics() {
        let mut builder = MirBuilder::default();
        let fn_id = builder.reserve_function();
        builder.seal_function(fn_id, trivial_builder());
        builder.seal_function(fn_id, trivial_builder());
    }

    fn trivial_builder() -> FunctionBuilder {
        let mut fb = new_builder();
        let block = fb.new_block();
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        fb
    }

    // --- Display impls ---

    #[test]
    fn condition_display() {
        assert_eq!(format!("{}", Condition::Zero), "== 0");
        assert_eq!(format!("{}", Condition::NotZero), "!= 0");
    }

    #[test]
    fn block_id_display_has_bb_prefix() {
        let (_blocks, ids) = new_block_ids(1);
        assert!(format!("{}", ids[0]).starts_with(".bb"));
    }

    #[test]
    fn local_display_shows_name_and_index() {
        let mut locals: SlotMap<LocalId, Local> = SlotMap::with_key();
        let id = locals.insert(Local {
            ty: Ty::U8,
            is_return: false,
            defined: SparseSecondaryMap::new(),
            name: Some(ident("x")),
        });
        let text = format!("{}", locals[id].display(id));
        assert!(text.starts_with("x#"));
        assert!(!text.contains("RET"));
    }

    #[test]
    fn local_display_return_local_uses_ret_suffix() {
        let mut locals: SlotMap<LocalId, Local> = SlotMap::with_key();
        let id = locals.insert(Local {
            ty: Ty::U8,
            is_return: true,
            defined: SparseSecondaryMap::new(),
            name: None,
        });
        assert_eq!(format!("{}", locals[id].display(id)), "#RET");
    }

    #[test]
    fn local_display_with_ty_includes_ty() {
        let mut locals: SlotMap<LocalId, Local> = SlotMap::with_key();
        let id = locals.insert(Local {
            ty: Ty::U8,
            is_return: true,
            defined: SparseSecondaryMap::new(),
            name: None,
        });
        let registry = TyRegistry::default();
        assert_eq!(
            format!("{}", locals[id].display_with_ty(id, &registry)),
            "#RET: u8"
        );
    }

    #[test]
    fn operand_display_local_and_literal() {
        let mir = Mir::default();

        let mut fb = new_builder();
        let block = fb.new_block();
        let local = fb.new_local(Ty::U8);
        fb.push_instr(
            block,
            Instr::Copy {
                target: local,
                source: Operand::Literal(IntegerLiteral { value: 1 }),
            },
        );
        fb.set_terminator(block, Terminator::Return);
        fb.seal_block(block);
        let function = fb.build();

        let literal = Operand::Literal(IntegerLiteral { value: 42 });
        assert_eq!(format!("{}", literal.display(&mir, &function)), "42");

        let local_operand = Operand::Local(local);
        assert!(format!("{}", local_operand.display(&mir, &function)).starts_with('#'));
    }

    #[test]
    fn callee_display_fn_ptr_shows_function_name() {
        let mut builder = MirBuilder::default();
        let callee_id = builder.reserve_function();
        builder.seal_function(callee_id, {
            let mut fb = FunctionBuilder::new(Identifier::from("callee"), Ty::UNIT);
            let block = fb.new_block();
            fb.set_terminator(block, Terminator::Return);
            fb.seal_block(block);
            fb
        });
        let mir = builder.build();
        let caller = trivially_built_function();

        let callee = Callee::FnPtr(callee_id);
        assert_eq!(format!("{}", callee.display(&mir, &caller)), "callee");
    }

    #[test]
    fn display_field_projections() {
        let mir = MirBuilder::default().build();
        let function = trivially_built_function();

        let projection = PlaceProjection::Field(Identifier::from("field"));
        assert_eq!(format!("{}", projection.display(&mir, &function)), ".field");
    }

    #[test]
    fn display_index_projections() {
        let mir = MirBuilder::default().build();
        let function = trivially_built_function();

        let projection = PlaceProjection::Index(Operand::Literal(IntegerLiteral { value: 3 }));
        assert_eq!(format!("{}", projection.display(&mir, &function)), "[3]");
    }
}
