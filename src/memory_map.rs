//! The memory map of a built program: which memory it declares, where its code is, and the
//! names of both.
//!
//! What a program declares comes from the program itself: globals and constants from their
//! declarations, code from the encoded sections. Where the linker put it comes from the symbol
//! file `rgblink` writes. A [`MemoryLayout`] holds the first half, gathered while the program is
//! compiled, and [`MemoryLayout::resolve`] combines it with the symbol file into a
//! [`MemoryMap`].

use crate::codegen::encode::block_symbol_name;
use crate::codegen::{
    BOOTSTRAP_SYMBOL, CART_HEADER_END_ADDR, CART_HEADER_START_ADDR, COMPILER_SYMBOL_PREFIX,
    INITIAL_SP, INTERRUPT_VECTOR_ADDRS, RST_VECTOR_ADDRS,
};
use crate::num::u3;
use crate::ty::{Ty, TyRegistry};
use crate::{lir, mir};
use quokka_lang_obj::ObjFile;
use quokka_lang_opcodes::instruction_len;
use std::collections::HashMap;
use std::fmt;
use std::ops::RangeInclusive;

/// The first address of High RAM, which the stack shares with any global placed there.
const HRAM_START_ADDR: u16 = 0xFF80;

/// The memory map of a linked program.
///
/// Every list is ordered by address. Ranges may overlap: globals in video memory, OAM and the
/// I/O registers are allowed to share addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryMap {
    /// The declared `global`s, including the ones mapped onto I/O registers.
    pub globals: Vec<Global>,
    /// The declared `const`s which are stored in ROM. A constant small enough to be used as an
    /// immediate is not stored, and so is not listed.
    pub constants: Vec<DataRange>,
    /// The ROM the compiler fills in for itself, which is not code: the cartridge header and
    /// the padding after each jump-vector stub.
    pub reserved: Vec<DataRange>,
    /// The code sections.
    pub code: Vec<CodeSection>,
    /// The stack. It is the only RAM the compiler allocates for itself: spilled registers are
    /// kept in stack frames.
    pub stack: Stack,
    /// The addresses of the code the compiler emits around the program.
    pub landmarks: Landmarks,
}

/// A declared `global` and the memory it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub addr: u16,
    /// The size of the global's type. One which is not a whole number of bytes only defines
    /// the low bits of its last byte.
    pub bits: u32,
    /// The offset of each byte of the global which holds a value narrower than a byte, in
    /// order. Such a byte only defines its low bits, and is stored to by reading it, replacing
    /// those bits and writing it back.
    pub narrow: Vec<u16>,
}

impl Global {
    /// The number of bytes the global occupies.
    #[must_use]
    pub fn len(&self) -> u16 {
        byte_len(self.bits)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }
}

/// A named range of ROM which holds data rather than code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRange {
    pub name: String,
    pub addr: u16,
    pub len: u16,
}

/// A contiguous range of ROM which holds code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSection {
    pub addr: u16,
    pub len: u16,
    /// The functions and blocks in the section, ordered by address. A function's symbol is its
    /// name, and each of its other blocks is the function's name followed by the block's.
    pub symbols: Vec<Symbol>,
    /// The address of every instruction in the section, in order.
    pub instructions: Vec<u16>,
}

/// A named address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub addr: u16,
}

/// The memory the stack grows through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stack {
    /// The value of `SP` when the entrypoint is called, one past the stack's highest byte.
    pub initial_sp: u16,
    /// The lowest address the stack can reach before it runs into a declared global or out of
    /// High RAM.
    pub floor: u16,
}

/// The addresses of the code the compiler emits around the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landmarks {
    /// The cartridge header, whose first four bytes are code which jumps to the bootstrap.
    pub header: RangeInclusive<u16>,
    /// The bootstrap code, which sets up `SP` and calls the entrypoint.
    pub bootstrap: u16,
    /// The loop the bootstrap code spins in once the entrypoint returns.
    pub spin_loop: u16,
    /// The `RST` and interrupt vector stubs.
    pub vectors: Vec<Symbol>,
}

impl fmt::Display for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "globals:")?;
        for global in &self.globals {
            write!(f, "  ")?;
            write_range(f, global.addr, global.len())?;
            write!(f, " {} ({} bits", global.name, global.bits)?;
            // each run of consecutive narrow bytes is listed as a range of offsets
            let mut separator = ", narrow";
            for run in global.narrow.chunk_by(|a, b| a + 1 == *b) {
                match run {
                    [offset] => write!(f, "{separator} +{offset}")?,
                    [first, .., last] => write!(f, "{separator} +{first}..=+{last}")?,
                    [] => {}
                }
                separator = "";
            }
            writeln!(f, ")")?;
        }
        for (title, ranges) in [("constants", &self.constants), ("reserved", &self.reserved)] {
            writeln!(f, "{title}:")?;
            for range in ranges {
                write!(f, "  ")?;
                write_range(f, range.addr, range.len)?;
                writeln!(f, " {}", range.name)?;
            }
        }
        writeln!(f, "code:")?;
        for section in &self.code {
            write!(f, "  ")?;
            write_range(f, section.addr, section.len)?;
            // each symbol is followed by the instructions between it and the next one
            let mut symbols = section.symbols.iter().peekable();
            for &instruction in &section.instructions {
                while let Some(symbol) = symbols.next_if(|symbol| symbol.addr <= instruction) {
                    write!(f, "\n    {}:", symbol.name)?;
                }
                write!(f, " ${instruction:04X}")?;
            }
            for symbol in symbols {
                write!(f, "\n    {}:", symbol.name)?;
            }
            writeln!(f)?;
        }
        writeln!(f, "stack:")?;
        writeln!(
            f,
            "  ${:04X}..${:04X}",
            self.stack.floor, self.stack.initial_sp
        )?;
        writeln!(f, "landmarks:")?;
        writeln!(
            f,
            "  header ${:04X}..=${:04X}",
            self.landmarks.header.start(),
            self.landmarks.header.end()
        )?;
        writeln!(f, "  bootstrap ${:04X}", self.landmarks.bootstrap)?;
        writeln!(f, "  spin loop ${:04X}", self.landmarks.spin_loop)?;
        for vector in &self.landmarks.vectors {
            writeln!(f, "  vector ${:04X} {}", vector.addr, vector.name)?;
        }
        Ok(())
    }
}

fn write_range(f: &mut fmt::Formatter<'_>, addr: u16, len: u16) -> fmt::Result {
    match len {
        0 => write!(f, "${addr:04X} (empty)"),
        len => write!(f, "${addr:04X}..=${:04X}", addr + (len - 1)),
    }
}

fn byte_len(bits: u32) -> u16 {
    u16::try_from(bits.div_ceil(8)).expect("a type is at most 64KiB")
}

/// Adds the offset of every byte of a `ty` placed at `offset` which holds a value narrower
/// than a byte. Each such value has a byte to itself, as array items and struct fields are
/// padded to whole bytes.
fn narrow_offsets(ty: Ty, ty_registry: &TyRegistry, offset: u16, narrow: &mut Vec<u16>) {
    if ty.is_array() {
        let data = ty_registry.array_ty_data(ty);
        let item_len = data.item_ty.bit_size(ty_registry).padded_bytes();
        let start = narrow.len();
        narrow_offsets(data.item_ty, ty_registry, offset, narrow);
        // every item is narrow at the same offsets as the first
        let item_narrow = narrow[start..].to_vec();
        for index in 1..data.count.get() {
            narrow.extend(item_narrow.iter().map(|first| first + index * item_len));
        }
    } else if ty.is_struct() {
        for field in ty_registry.struct_ty_data(ty).fields.values() {
            narrow_offsets(
                field.ty,
                ty_registry,
                offset + field.bit_offset.bytes(),
                narrow,
            );
        }
    } else if ty.bit_size(ty_registry).bits() != u3::new(0) {
        narrow.push(offset);
    }
}

/// What a program declares, before the linker has placed it.
#[derive(Debug, Clone, Default)]
pub struct MemoryLayout {
    globals: Vec<DeclaredGlobal>,
    reserved: Vec<UnplacedData>,
    code: Vec<UnplacedCode>,
    /// The name of the symbol at the bootstrap code's spin loop.
    spin_loop: Option<String>,
}

/// A `global` or `const` as declared in source.
#[derive(Debug, Clone)]
pub struct DeclaredGlobal {
    name: String,
    bits: u32,
    /// The offset of each byte which holds a value narrower than a byte.
    narrow: Vec<u16>,
    mutable: bool,
    /// The address the declaration pins it to. The linker chooses otherwise.
    addr: Option<u16>,
}

impl DeclaredGlobal {
    /// The declared globals of `mir`, in declaration order.
    #[must_use]
    pub fn from_mir(mir: &mir::Mir, ty_registry: &TyRegistry) -> Vec<Self> {
        mir.globals
            .values()
            .map(|global| DeclaredGlobal {
                name: global.name.data.to_string(),
                bits: global.ty.bit_size(ty_registry).into(),
                narrow: {
                    let mut narrow = Vec::new();
                    narrow_offsets(global.ty, ty_registry, 0, &mut narrow);
                    narrow
                },
                mutable: global.is_mutable(),
                addr: match &global.attrs {
                    mir::GlobalAttributes::Rom(attrs) => attrs.address,
                    mir::GlobalAttributes::Ram(attrs) => attrs.address,
                },
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct UnplacedData {
    name: String,
    addr: Option<u16>,
    len: u32,
}

#[derive(Debug, Clone)]
struct UnplacedCode {
    /// The address the section is pinned to. The linker chooses otherwise.
    addr: Option<u16>,
    len: u32,
    /// Offset into the section and name of each function and block in it, ordered by offset.
    symbols: Vec<(u32, String)>,
    /// Offset into the section of each instruction.
    instructions: Vec<u32>,
}

impl MemoryLayout {
    /// Gathers the layout of a program from its declared `globals`, the `lir` it was lowered
    /// to, and the `obj` file that was encoded to.
    #[must_use]
    pub fn new(globals: Vec<DeclaredGlobal>, lir: &lir::Lir, obj: &ObjFile) -> Self {
        let mut layout = MemoryLayout {
            globals,
            ..MemoryLayout::default()
        };
        for section in lir.sections.values() {
            match &section.data {
                lir::SectionData::Code(code) => {
                    if let Some(code) = unplaced_code(code, obj) {
                        layout.code.push(code);
                    }
                    if let Some(spin_loop) = spin_loop_symbol(code) {
                        layout.spin_loop = Some(spin_loop);
                    }
                }
                lir::SectionData::Data(data) => {
                    // the other data sections are declared constants, which come from `globals`
                    if data.name.as_str().starts_with(COMPILER_SYMBOL_PREFIX) {
                        layout.reserved.push(UnplacedData {
                            name: data.name.to_string(),
                            addr: data.addr,
                            len: u32::try_from(data.data.len()).expect("u32 overflowed"),
                        });
                    }
                }
                // RAM sections are declared globals, which come from `globals`
                lir::SectionData::Ram(_) => {}
            }
        }
        layout
    }

    /// Places the layout using `symbols`, the addresses the linker assigned.
    pub fn resolve(&self, symbols: &SymbolTable) -> Result<MemoryMap, MemoryMapError> {
        let mut globals = Vec::new();
        let mut constants = Vec::new();
        for global in &self.globals {
            let addr = match global.addr {
                Some(addr) => addr,
                None => symbols.addr(&global.name)?,
            };
            check_end(&global.name, addr, u32::from(byte_len(global.bits)))?;
            if global.mutable {
                globals.push(Global {
                    name: global.name.clone(),
                    addr,
                    bits: global.bits,
                    narrow: global.narrow.clone(),
                });
            } else {
                constants.push(DataRange {
                    name: global.name.clone(),
                    addr,
                    len: byte_len(global.bits),
                });
            }
        }
        globals.sort_by(|a, b| (a.addr, &a.name).cmp(&(b.addr, &b.name)));
        constants.sort_by(|a, b| (a.addr, &a.name).cmp(&(b.addr, &b.name)));

        let mut reserved = Vec::new();
        for data in &self.reserved {
            let addr = match data.addr {
                Some(addr) => addr,
                None => symbols.addr(&data.name)?,
            };
            reserved.push(DataRange {
                name: data.name.clone(),
                addr,
                len: check_end(&data.name, addr, data.len)?,
            });
        }
        reserved.sort_by_key(|range| range.addr);

        let mut code = Vec::new();
        for section in &self.code {
            code.push(section.resolve(symbols)?);
        }
        code.sort_by_key(|section| section.addr);

        let label = |name: &str| {
            code.iter()
                .flat_map(|section| &section.symbols)
                .find(|symbol| symbol.name == name)
                .map(|symbol| symbol.addr)
                .ok_or_else(|| MemoryMapError::MissingSymbol {
                    name: name.to_string(),
                })
        };
        let spin_loop = match &self.spin_loop {
            Some(name) => label(name)?,
            None => {
                return Err(MemoryMapError::MissingSymbol {
                    name: format!("{BOOTSTRAP_SYMBOL} spin loop"),
                });
            }
        };
        let landmarks = Landmarks {
            header: CART_HEADER_START_ADDR..=CART_HEADER_END_ADDR,
            bootstrap: label(BOOTSTRAP_SYMBOL)?,
            spin_loop,
            vectors: RST_VECTOR_ADDRS
                .iter()
                .chain(&INTERRUPT_VECTOR_ADDRS)
                .map(|&(addr, name)| Symbol {
                    name: name.to_string(),
                    addr,
                })
                .collect(),
        };

        let floor = globals
            .iter()
            .filter(|global| (HRAM_START_ADDR..INITIAL_SP).contains(&global.addr))
            .map(|global| global.addr.saturating_add(global.len()).min(INITIAL_SP))
            .max()
            .unwrap_or(HRAM_START_ADDR);

        Ok(MemoryMap {
            globals,
            constants,
            reserved,
            code,
            stack: Stack {
                initial_sp: INITIAL_SP,
                floor,
            },
            landmarks,
        })
    }
}

impl UnplacedCode {
    fn resolve(&self, symbols: &SymbolTable) -> Result<CodeSection, MemoryMapError> {
        let name = self
            .symbols
            .first()
            .map_or("<code section>", |(_, name)| name.as_str());
        let base = match self.addr {
            Some(addr) => addr,
            None => {
                // any of the section's symbols the linker lists gives away where it starts
                let (offset, addr) = self
                    .symbols
                    .iter()
                    .find_map(|(offset, name)| Some((*offset, symbols.get(name)?)))
                    .ok_or_else(|| MemoryMapError::MissingSymbol {
                        name: name.to_string(),
                    })?;
                u32::from(addr)
                    .checked_sub(offset)
                    .and_then(|base| u16::try_from(base).ok())
                    .ok_or_else(|| MemoryMapError::OutOfRange {
                        name: name.to_string(),
                    })?
            }
        };
        let len = check_end(name, base, self.len)?;
        // every offset is within the section, which `check_end` found to fit the address space
        #[allow(clippy::cast_possible_truncation)]
        let place = |offset: u32| base + offset as u16;
        Ok(CodeSection {
            addr: base,
            len,
            symbols: self
                .symbols
                .iter()
                .map(|(offset, name)| Symbol {
                    name: name.clone(),
                    addr: place(*offset),
                })
                .collect(),
            instructions: self.instructions.iter().copied().map(place).collect(),
        })
    }
}

/// Checks that `len` bytes at `addr` fit in the address space, returning `len`.
fn check_end(name: &str, addr: u16, len: u32) -> Result<u16, MemoryMapError> {
    u16::try_from(len)
        .ok()
        .filter(|_| u32::from(addr) + len <= 0x1_0000)
        .ok_or_else(|| MemoryMapError::OutOfRange {
            name: name.to_string(),
        })
}

/// The layout of `code`, found through the `obj` section holding its functions' symbols. A
/// section with no function has no symbol to find it by, and no way to be reached.
fn unplaced_code(code: &lir::CodeSectionData, obj: &ObjFile) -> Option<UnplacedCode> {
    let obj_section = code.functions.values().find_map(|function| {
        let function = function.as_str();
        obj.symbols
            .iter()
            .find(|symbol| symbol.name == function)?
            .data
            .as_definition()?
            .section
    })?;
    let bytes = &obj.sections[obj_section.index()].data.as_rom_data()?.bytes;

    let mut symbols: Vec<(u32, String)> = obj
        .symbols
        .iter()
        .filter_map(|symbol| {
            let definition = symbol.data.as_definition()?;
            (definition.section == Some(obj_section))
                .then(|| (definition.value, symbol.name.clone()))
        })
        .collect();
    symbols.sort();

    // a code section holds nothing but instructions, so walking it from the start by each
    // instruction's length finds every one of them
    let mut instructions = Vec::new();
    let mut offset = 0;
    while let Some(&opcode) = bytes.get(offset) {
        instructions.push(u32::try_from(offset).expect("u32 overflowed"));
        offset += usize::from(instruction_len(opcode));
    }
    debug_assert_eq!(offset, bytes.len(), "code section ends mid-instruction");

    Some(UnplacedCode {
        addr: code.addr,
        len: u32::try_from(bytes.len()).expect("u32 overflowed"),
        symbols,
        instructions,
    })
}

/// The name of the symbol at the spin loop, if `code` is the bootstrap code: the block the
/// call to the entrypoint returns to.
fn spin_loop_symbol(code: &lir::CodeSectionData) -> Option<String> {
    let (entry, function) = code
        .functions
        .iter()
        .find(|(_, function)| function.as_str() == BOOTSTRAP_SYMBOL)?;
    match &code.blocks[entry].terminator {
        lir::Terminator::Call { return_block, .. } => {
            Some(block_symbol_name(function, *return_block))
        }
        _ => None,
    }
}

/// The addresses the linker assigned, read from the symbol file `rgblink` writes.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    addrs: HashMap<String, u16>,
}

impl SymbolTable {
    /// Parses the contents of a symbol file: a `bank:address name` line per symbol, with `;`
    /// starting a comment.
    pub fn parse(contents: &str) -> Result<Self, MemoryMapError> {
        let mut addrs = HashMap::new();
        for (index, line) in contents.lines().enumerate() {
            let line = line.split(';').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let malformed = || MemoryMapError::MalformedSymbolFile { line: index + 1 };
            let (location, name) = line.split_once(char::is_whitespace).ok_or_else(malformed)?;
            let (_bank, addr) = location.split_once(':').ok_or_else(malformed)?;
            let addr = u16::from_str_radix(addr, 16).map_err(|_| malformed())?;
            let name = unescape_symbol_name(name.trim()).ok_or_else(malformed)?;
            addrs.insert(name, addr);
        }
        Ok(SymbolTable { addrs })
    }

    /// The address of the symbol called `name`, if the linker listed it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<u16> {
        self.addrs.get(name).copied()
    }

    fn addr(&self, name: &str) -> Result<u16, MemoryMapError> {
        self.get(name).ok_or_else(|| MemoryMapError::MissingSymbol {
            name: name.to_string(),
        })
    }
}

/// Undoes the escaping `rgblink` applies to a symbol name: any character other than an ASCII
/// letter, a digit or one of `_@#$.` is written as `\uXXXX` or `\UXXXXXXXX`.
fn unescape_symbol_name(name: &str) -> Option<String> {
    let mut result = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(index) = rest.find('\\') {
        result.push_str(&rest[..index]);
        let digits = match rest.as_bytes().get(index + 1)? {
            b'u' => 4,
            b'U' => 8,
            _ => return None,
        };
        let code = rest.get(index + 2..index + 2 + digits)?;
        result.push(char::from_u32(u32::from_str_radix(code, 16).ok()?)?);
        rest = &rest[index + 2 + digits..];
    }
    result.push_str(rest);
    Some(result)
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum MemoryMapError {
    #[error("the symbol file is malformed at line {line}")]
    MalformedSymbolFile { line: usize },
    #[error("the symbol file does not list `{name}`")]
    MissingSymbol { name: String },
    #[error("`{name}` does not fit in the address space")]
    OutOfRange { name: String },
}
