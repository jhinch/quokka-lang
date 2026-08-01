use crate::ast::{Identifier, IntegerLiteral, SpannedIdentifier};
use crate::ty::Ty;
use slotmap::{SecondaryMap, SlotMap, SparseSecondaryMap, new_key_type};
use std::collections::HashMap;

new_key_type! { pub struct GlobalId; }
new_key_type! { pub struct FunctionId; }
new_key_type! { pub struct BlockId; }
new_key_type! { pub struct LocalId; }

/// Middle Intermediate Representation (MIR)
#[derive(Debug, Clone, Default)]
pub struct Mir {
    pub globals: SlotMap<GlobalId, Global>,
    pub functions: SlotMap<FunctionId, Function>,
}

/// MIR function represented as a control flow graph in static single assignment form
#[derive(Debug, Clone, Default)]
pub struct Function {
    pub name: Identifier,
    pub return_ty: Ty,
    pub return_local: Option<LocalId>,
    pub blocks: SlotMap<BlockId, Block>,
    pub locals: SlotMap<LocalId, Local>,
    pub start_block: BlockId,
    pub end_blocks: Vec<BlockId>,
}

/// MIR Basic block
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub instructions: Vec<Instr>,
    pub terminator: Terminator,
    pub previous: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct Global {
    pub ty: Ty,
    pub name: SpannedIdentifier,
}

#[derive(Debug, Clone)]
pub struct Local {
    pub ty: Ty,
    pub defined: Option<(BlockId, usize)>,
    pub name: Option<SpannedIdentifier>,
}

/// A MIR Instruction
#[derive(Debug, Clone)]
pub enum Instr {
    // assignment
    ReturnValue {
        target: LocalId,
    },
    Phi {
        target: LocalId,
        sources: SparseSecondaryMap<BlockId, LocalId>,
    },
    Load {
        target: LocalId,
        source: GlobalPlace,
    },
    Store {
        target: GlobalPlace,
        source: Operand,
    },
    Copy {
        target: LocalId,
        source: Operand,
    },
    // unary
    Not {
        target: LocalId,
        source: Operand,
    },
    Neg {
        target: LocalId,
        source: Operand,
    },
    // comparison
    Eq {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Ne {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Lt {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Le {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Ge {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Gt {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    // bitwise
    BitAnd {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    BitOr {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    BitXor {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    RotateCLeft {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    RotateCRight {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    RotateLeft {
        target: LocalId,
        source: Operand,
    },
    RotateRight {
        target: LocalId,
        source: Operand,
    },
    ShiftLeft {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    ShiftRight {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    // arithmetic
    Add {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
    Sub {
        target: LocalId,
        left: Operand,
        right: Operand,
    },
}

impl Instr {
    pub fn target_local(&self) -> Option<LocalId> {
        match self {
            Instr::ReturnValue { target, .. }
            | Instr::Phi { target, .. }
            | Instr::Load { target, .. }
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
            | Instr::RotateCLeft { target, .. }
            | Instr::RotateCRight { target, .. }
            | Instr::RotateLeft { target, .. }
            | Instr::RotateRight { target, .. }
            | Instr::ShiftLeft { target, .. }
            | Instr::ShiftRight { target, .. }
            | Instr::Add { target, .. }
            | Instr::Sub { target, .. } => Some(*target),
            Instr::Store { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Return,
    Call {
        callee: Callee,
        return_block: BlockId,
    },
    CallIf {
        source: LocalId,
        condition: Condition,
        callee: Callee,
        return_block: BlockId,
        else_block: BlockId,
    },
    Jump {
        target_block: BlockId,
    },
    JumpIf {
        source: LocalId,
        condition: Condition,
        then_block: BlockId,
        else_block: BlockId,
    },
    ReturnIf {
        source: LocalId,
        condition: Condition,
        else_block: BlockId,
    },
}

impl Default for Terminator {
    fn default() -> Self {
        Terminator::Return
    }
}

#[derive(Debug, Clone)]
pub enum Callee {
    Local(LocalId),
    FnPtr(FunctionId),
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(LocalId),
    Literal(IntegerLiteral),
    FnPtr(FunctionId),
}

#[derive(Debug, Clone)]
pub struct GlobalPlace {
    pub global: GlobalId,
    pub projections: Vec<GlobalPlaceProjection>,
}

#[derive(Debug, Clone)]
pub enum GlobalPlaceProjection {
    Field(Identifier),
    Index(Operand),
}

#[derive(Debug, Clone, Copy)]
pub enum Condition {
    Zero,
    NotZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variable(u32);

#[derive(Default)]
pub struct MirBuilder {
    mir: Mir,
    unsealed_functions: SecondaryMap<FunctionId, ()>,
}

impl MirBuilder {
    pub fn insert_global(&mut self, name: SpannedIdentifier, ty: Ty) -> GlobalId {
        let global = self.mir.globals.insert(Global { name, ty });
        global
    }

    pub fn reserve_function(&mut self) -> FunctionId {
        let function = self.mir.functions.insert(Function::default());
        self.unsealed_functions.insert(function, ());
        function
    }

    pub fn seal_function(&mut self, function_id: FunctionId, function: Function) {
        self.unsealed_functions
            .remove(function_id)
            .expect("Function already sealed");
        self.mir.functions[function_id] = function;
    }

    #[must_use]
    pub fn build(mut self) -> Mir {
        for function in self.unsealed_functions.keys() {
            self.mir.functions.remove(function);
        }
        self.mir
    }
}

pub struct FunctionBuilder {
    function: Function,
    variable_assignments: HashMap<Variable, HashMap<BlockId, LocalId>>,
    variable_names: HashMap<Variable, SpannedIdentifier>,
    terminated_blocks: SecondaryMap<BlockId, ()>,
    sealed_blocks: SecondaryMap<BlockId, ()>,
    incomplete_phis: HashMap<BlockId, HashMap<Variable, usize>>,
    next_var: u32,
}

impl FunctionBuilder {
    #[must_use]
    pub fn new(return_ty: Ty) -> Self {
        FunctionBuilder {
            function: Function {
                return_ty,
                ..Default::default()
            },
            variable_assignments: HashMap::new(),
            variable_names: HashMap::new(),
            terminated_blocks: SecondaryMap::new(),
            sealed_blocks: SecondaryMap::new(),
            incomplete_phis: HashMap::new(),
            next_var: 0,
        }
    }

    #[must_use]
    pub fn build(mut self) -> Function {
        let blocks = self.function.blocks.keys().collect::<Vec<_>>();
        for block in blocks {
            if !self.sealed_blocks.contains_key(block) {
                self.seal_block(block);
            }
        }
        // remove all dangling blocks
        for block in self.function.blocks.keys().collect::<Vec<_>>() {
            if self.function.start_block != block && self.function.blocks[block].previous.is_empty()
            {
                self.function.blocks.remove(block);
            }
        }
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

    pub fn push_local(&mut self, ty: Ty) -> LocalId {
        self.function.locals.insert(Local {
            ty,
            defined: None,
            name: None,
        })
    }

    pub fn push_return_local(&mut self, ty: Ty) -> LocalId {
        let local = self.function.locals.insert(Local {
            ty,
            defined: None,
            name: None,
        });
        self.function.return_local = Some(local);
        local
    }

    pub fn push_instr(&mut self, block: BlockId, instr: Instr) -> usize {
        let target = instr.target_local();
        let instructions = &mut self.function.blocks[block].instructions;
        let index = instructions.len();
        instructions.push(instr);
        if let Some(target) = target {
            if self.function.return_local != Some(target) {
                let previous = self.function.locals[target].defined.replace((block, index));
                debug_assert!(
                    previous.is_none(),
                    "local {target:?} defined multiple times. {previous:?}, Some(({block:?}, {index:?}))"
                );
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
                self.function.end_blocks.push(block);
            }
            Terminator::ReturnIf {
                else_block: target_block,
                ..
            } => {
                self.function.blocks[*target_block].previous.push(block);
                self.function.end_blocks.push(block);
            }
            Terminator::Call {
                return_block: target_block,
                ..
            }
            | Terminator::Jump { target_block, .. } => {
                self.function.blocks[*target_block].previous.push(block);
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
                self.function.blocks[*then_block].previous.push(block);
                self.function.blocks[*else_block].previous.push(block);
            }
        }
        self.function.blocks[block].terminator = terminator;
        self.terminated_blocks.insert(block, ());
    }

    pub fn write_variable(&mut self, var: Variable, block: BlockId, local: LocalId) {
        self.variable_assignments
            .entry(var)
            .or_default()
            .insert(block, local);
        if let Some(name) = self.variable_names.get(&var) {
            if let Some(previous) = self.function.locals[local].name.as_ref() {
                debug_assert!(
                    previous == name,
                    "cannot name {local:?} {name:?}, already named {previous:?}"
                );
            }
            self.function.locals[local].name = Some(name.clone());
        }
    }

    pub fn read_variable(&mut self, var: Variable, block: BlockId) -> LocalId {
        if let Some(value) = self
            .variable_assignments
            .get(&var)
            .and_then(|assignments| assignments.get(&block))
        {
            return *value;
        }
        self.read_variable_recursive(var, block)
    }

    fn read_variable_recursive(&mut self, var: Variable, block: BlockId) -> LocalId {
        let value = if !self.sealed_blocks.contains_key(block) {
            let target = self.push_local(Ty::UNRESOLVED);
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
            let previous_blocks = self.function.blocks[block].previous.clone();
            match previous_blocks.as_slice() {
                [] => {
                    panic!("attempted to read variable {var:?} from dangling block {block:?}")
                }
                [single] => self.read_variable(var, *single),
                _ => {
                    let target = self.push_local(Ty::UNRESOLVED);
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
        let previous_blocks = self.function.blocks[block].previous.clone();
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
        while let Some((block, index)) = self.function.locals[local].defined {
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
