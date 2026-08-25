use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Reg {
    A = 0x0001,
    B = 0x0002,
    C = 0x0004,
    D = 0x0008,
    E = 0x0010,
    H = 0x0020,
    L = 0x0040,
    SP = 0x0080,
    Fc = 0x0100,
    Fz = 0x0200,
}

impl TryFrom<u16> for Reg {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Ok(match value {
            0x0001 => Reg::A,
            0x0002 => Reg::B,
            0x0004 => Reg::C,
            0x0008 => Reg::D,
            0x0010 => Reg::E,
            0x0020 => Reg::H,
            0x0040 => Reg::L,
            0x0080 => Reg::SP,
            0x0100 => Reg::Fc,
            0x0200 => Reg::Fz,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Color {
    A = 0x0001,
    B = 0x0002,
    C = 0x0004,
    D = 0x0008,
    E = 0x0010,
    H = 0x0020,
    L = 0x0040,
    AF = 0x0080,
    BC = 0x0100,
    DE = 0x0200,
    HL = 0x0400,
    SP = 0x0800,
    Fz = 0x1000,
    Fc = 0x2000,
}

impl Color {
    /// Accumulator
    #[must_use]
    pub const fn acc() -> &'static [Color] {
        &[Color::A]
    }

    /// general purposes 8-bit registers
    #[must_use]
    pub const fn r8() -> &'static [Color] {
        &[
            Color::A,
            Color::B,
            Color::C,
            Color::D,
            Color::E,
            Color::H,
            Color::L,
        ]
    }

    /// general purpose 16-bit register
    #[must_use]
    pub const fn r16() -> &'static [Color] {
        &[Color::BC, Color::DE, Color::HL]
    }

    /// 16-bit register which you can PUSH and POP from the stack
    #[must_use]
    pub const fn r16_stack() -> &'static [Color] {
        &[Color::BC, Color::DE, Color::AF]
    }

    /// 16-bit register which can be used as a general purpose pointer to memory
    #[must_use]
    pub const fn r16_ptr() -> &'static [Color] {
        &[Color::HL]
    }

    /// 8-bit register which can be used as a pointer to memory in the `$FFxx` range.
    #[must_use]
    pub const fn r8_high_ptr() -> &'static [Color] {
        &[Color::C]
    }

    /// 16-bit register which can be used as a pointers to memory during a `LD`
    #[must_use]
    pub const fn r16_ptr_ld() -> &'static [Color] {
        &[Color::BC, Color::DE, Color::HL]
    }

    /// flags
    #[must_use]
    pub const fn flags() -> &'static [Color] {
        &[Color::Fz, Color::Fc]
    }

    /// zero flag
    #[must_use]
    pub const fn z() -> &'static [Color] {
        &[Color::Fz]
    }

    /// carry flag
    #[must_use]
    pub const fn c() -> &'static [Color] {
        &[Color::Fc]
    }
}

impl Color {
    #[must_use]
    pub const fn regs(self) -> RegSet {
        match self {
            Color::A => RegSet::just(Reg::A),
            Color::B => RegSet::just(Reg::B),
            Color::C => RegSet::just(Reg::C),
            Color::D => RegSet::just(Reg::D),
            Color::E => RegSet::just(Reg::E),
            Color::H => RegSet::just(Reg::H),
            Color::L => RegSet::just(Reg::L),
            Color::BC => RegSet::just_2(Reg::B, Reg::C),
            Color::DE => RegSet::just_2(Reg::D, Reg::E),
            Color::HL => RegSet::just_2(Reg::H, Reg::L),
            Color::AF => RegSet::just_3(Reg::A, Reg::Fz, Reg::Fc),
            Color::SP => RegSet::just(Reg::SP),
            Color::Fz => RegSet::just(Reg::Fz),
            Color::Fc => RegSet::just(Reg::Fc),
        }
    }

    #[must_use]
    pub fn conflicts(self, other: Color) -> bool {
        !(self.regs() & other.regs()).is_empty()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RegSet {
    bits: u16,
}

impl RegSet {
    #[must_use]
    pub const fn empty() -> Self {
        RegSet { bits: 0 }
    }

    const fn just(reg: Reg) -> Self {
        RegSet { bits: reg as u16 }
    }

    const fn just_2(reg1: Reg, reg2: Reg) -> Self {
        RegSet {
            bits: reg1 as u16 | reg2 as u16,
        }
    }

    const fn just_3(reg1: Reg, reg2: Reg, reg3: Reg) -> Self {
        RegSet {
            bits: reg1 as u16 | reg2 as u16 | reg3 as u16,
        }
    }
}

impl RegSet {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    #[must_use]
    pub const fn contains(&self, value: Reg) -> bool {
        self.bits & value as u16 != 0
    }

    pub fn insert(&mut self, value: Reg) -> bool {
        let bits = value as u16;
        if self.bits & bits != 0 {
            return false;
        }
        self.bits |= bits;
        true
    }

    pub fn remove(&mut self, value: Reg) -> bool {
        let bits = value as u16;
        if self.bits & bits == 0 {
            return false;
        }
        self.bits &= !bits;
        true
    }
}

impl fmt::Debug for RegSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.debug_tuple("RegSet")
                .field(&format_args!("{:b}", self.bits))
                .finish()
        } else {
            let mut debug_set = f.debug_set();
            for reg in self.clone().into_iter() {
                debug_set.entry(&reg);
            }
            debug_set.finish()
        }
    }
}

impl BitAnd for RegSet {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        RegSet {
            bits: self.bits & rhs.bits,
        }
    }
}

impl BitAndAssign for RegSet {
    fn bitand_assign(&mut self, rhs: Self) {
        self.bits &= rhs.bits;
    }
}

impl BitOr for RegSet {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        RegSet {
            bits: self.bits | rhs.bits,
        }
    }
}

impl BitOrAssign for RegSet {
    fn bitor_assign(&mut self, rhs: Self) {
        self.bits |= rhs.bits;
    }
}

impl IntoIterator for RegSet {
    type Item = Reg;
    type IntoIter = RegSetIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        RegSetIntoIter(self)
    }
}

pub struct RegSetIntoIter(RegSet);

impl Iterator for RegSetIntoIter {
    type Item = Reg;

    fn next(&mut self) -> Option<Self::Item> {
        let value = u16::try_from(self.0.bits.trailing_zeros()).ok()?;
        let reg = Reg::try_from(value).ok()?;
        self.0.remove(reg);
        Some(reg)
    }
}
