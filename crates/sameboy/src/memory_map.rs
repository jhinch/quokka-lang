//! The memory a program declares, for checkers to judge its execution against and for naming
//! the addresses in what they report.

use std::collections::BTreeSet;
use std::fmt;

/// The memory a program declares: the regions it may touch, where its instructions start, and
/// the names of its code.
///
/// The emulator knows nothing about the program it runs, so whoever built the ROM describes it
/// by adding to an empty map, and hands it over with
/// [`Sameboy::set_memory_map`](crate::Sameboy::set_memory_map).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryMap {
    regions: Vec<Region>,
    instructions: BTreeSet<u16>,
    narrow: BTreeSet<u16>,
    /// Sorted by address.
    labels: Vec<(u16, String)>,
    spin_loop: Option<u16>,
}

/// A named range of memory the program declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub kind: RegionKind,
    pub name: String,
    pub start: u16,
    pub len: u16,
}

impl Region {
    /// Whether `addr` is within the region.
    #[must_use]
    pub fn contains(&self, addr: u16) -> bool {
        addr >= self.start && addr - self.start < self.len
    }
}

/// What a [`Region`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Instructions.
    Code,
    /// Read-only data the program declares.
    Constant,
    /// ROM that belongs to neither the program's code nor its data, such as the cartridge
    /// header.
    Reserved,
    /// Memory the program declares for reading and writing, including I/O registers.
    Global,
    /// The memory the stack grows through. It ends at the initial value of `SP`.
    Stack,
}

/// An address described relative to the name it falls under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbolized<'a> {
    pub name: &'a str,
    pub offset: u16,
}

impl fmt::Display for Symbolized<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.offset {
            0 => write!(f, "{}", self.name),
            offset => write!(f, "{}+{offset}", self.name),
        }
    }
}

impl MemoryMap {
    /// A map which declares nothing.
    #[must_use]
    pub fn new() -> Self {
        MemoryMap::default()
    }

    /// Declare a region. Regions may overlap.
    pub fn add_region(&mut self, region: Region) {
        self.regions.push(region);
    }

    /// Declare that an instruction starts at `addr`.
    pub fn add_instruction(&mut self, addr: u16) {
        self.instructions.insert(addr);
    }

    /// Declare that the byte at `addr` holds a value narrower than a byte, which only defines
    /// some of its bits.
    pub fn add_narrow(&mut self, addr: u16) {
        self.narrow.insert(addr);
    }

    /// Name the code at `addr`, up to the next label.
    pub fn add_label(&mut self, addr: u16, name: impl Into<String>) {
        let index = self.labels.partition_point(|(other, _)| *other <= addr);
        self.labels.insert(index, (addr, name.into()));
    }

    /// Declare the address of the loop the program spins in once it has finished.
    pub fn set_spin_loop(&mut self, addr: u16) {
        self.spin_loop = Some(addr);
    }

    /// Every declared region, in the order they were added.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The regions `addr` is within.
    pub fn regions_at(&self, addr: u16) -> impl Iterator<Item = &Region> {
        self.regions
            .iter()
            .filter(move |region| region.contains(addr))
    }

    /// Whether `addr` is within a region of the given `kind`.
    #[must_use]
    pub fn is_within(&self, kind: RegionKind, addr: u16) -> bool {
        self.regions_at(addr).any(|region| region.kind == kind)
    }

    /// Whether an instruction starts at `addr`.
    #[must_use]
    pub fn is_instruction(&self, addr: u16) -> bool {
        self.instructions.contains(&addr)
    }

    /// Whether the byte at `addr` holds a value narrower than a byte.
    #[must_use]
    pub fn is_narrow(&self, addr: u16) -> bool {
        self.narrow.contains(&addr)
    }

    /// The address of the loop the program spins in once it has finished.
    #[must_use]
    pub fn spin_loop(&self) -> Option<u16> {
        self.spin_loop
    }

    /// Describe `addr` by name: the nearest label at or before it for an address in code, and
    /// the smallest region holding it otherwise.
    #[must_use]
    pub fn symbolize(&self, addr: u16) -> Option<Symbolized<'_>> {
        let code = self
            .regions_at(addr)
            .find(|region| region.kind == RegionKind::Code);
        if let Some(code) = code {
            let preceding = self.labels.partition_point(|(label, _)| *label <= addr);
            if let Some((label, name)) = self.labels[..preceding].last()
                && code.contains(*label)
            {
                return Some(Symbolized {
                    name,
                    offset: addr - label,
                });
            }
        }
        self.regions_at(addr)
            .filter(|region| !region.name.is_empty())
            .min_by_key(|region| region.len)
            .map(|region| Symbolized {
                name: &region.name,
                offset: addr - region.start,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(kind: RegionKind, name: &str, start: u16, len: u16) -> Region {
        Region {
            kind,
            name: name.to_string(),
            start,
            len,
        }
    }

    fn symbolized(map: &MemoryMap, addr: u16) -> Option<String> {
        map.symbolize(addr).map(|symbol| symbol.to_string())
    }

    #[test]
    fn region_contains_its_first_and_last_byte_only() {
        let region = region(RegionKind::Global, "G", 0xC000, 2);
        assert!(!region.contains(0xBFFF));
        assert!(region.contains(0xC000));
        assert!(region.contains(0xC001));
        assert!(!region.contains(0xC002));
    }

    #[test]
    fn region_at_the_top_of_memory_does_not_overflow() {
        let region = region(RegionKind::Global, "IE", 0xFFFF, 1);
        assert!(region.contains(0xFFFF));
        assert!(!region.contains(0xFFFE));
    }

    #[test]
    fn empty_region_contains_nothing() {
        assert!(!region(RegionKind::Global, "EMPTY", 0xC000, 0).contains(0xC000));
    }

    #[test]
    fn overlapping_regions_are_all_found() {
        let mut map = MemoryMap::new();
        map.add_region(region(RegionKind::Global, "SPRITES", 0xFE00, 160));
        map.add_region(region(RegionKind::Global, "FIRST", 0xFE00, 4));
        let names: Vec<&str> = map.regions_at(0xFE01).map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["SPRITES", "FIRST"]);
        assert!(map.is_within(RegionKind::Global, 0xFE01));
        assert!(!map.is_within(RegionKind::Stack, 0xFE01));
        assert!(!map.is_within(RegionKind::Global, 0xFEA0));
    }

    #[test]
    fn instruction_starts_are_exact() {
        let mut map = MemoryMap::new();
        map.add_instruction(0x0150);
        map.add_instruction(0x0153);
        assert!(map.is_instruction(0x0150));
        assert!(!map.is_instruction(0x0151));
        assert!(map.is_instruction(0x0153));
    }

    #[test]
    fn narrow_bytes_are_exact() {
        let mut map = MemoryMap::new();
        map.add_narrow(0xC001);
        assert!(!map.is_narrow(0xC000));
        assert!(map.is_narrow(0xC001));
        assert!(!map.is_narrow(0xC002));
    }

    #[test]
    fn code_is_symbolized_by_the_nearest_preceding_label() {
        let mut map = MemoryMap::new();
        map.add_region(region(RegionKind::Code, "", 0x0150, 0x20));
        // added out of order
        map.add_label(0x0160, "main.bb2");
        map.add_label(0x0150, "main");
        assert_eq!(symbolized(&map, 0x0150).as_deref(), Some("main"));
        assert_eq!(symbolized(&map, 0x015F).as_deref(), Some("main+15"));
        assert_eq!(symbolized(&map, 0x0160).as_deref(), Some("main.bb2"));
        assert_eq!(symbolized(&map, 0x016F).as_deref(), Some("main.bb2+15"));
        assert_eq!(symbolized(&map, 0x0170), None);
    }

    #[test]
    fn label_from_another_code_region_is_not_used() {
        let mut map = MemoryMap::new();
        map.add_region(region(RegionKind::Code, "", 0x0150, 4));
        map.add_region(region(RegionKind::Code, "", 0x0200, 4));
        map.add_label(0x0150, "main");
        assert_eq!(symbolized(&map, 0x0201), None);
    }

    #[test]
    fn data_is_symbolized_by_the_smallest_region_holding_it() {
        let mut map = MemoryMap::new();
        map.add_region(region(RegionKind::Global, "SPRITES", 0xFE00, 160));
        map.add_region(region(RegionKind::Global, "FIRST", 0xFE00, 4));
        assert_eq!(symbolized(&map, 0xFE02).as_deref(), Some("FIRST+2"));
        assert_eq!(symbolized(&map, 0xFE04).as_deref(), Some("SPRITES+4"));
        assert_eq!(symbolized(&map, 0xC000), None);
    }
}
