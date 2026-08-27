use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Reg {
    A = 1 << 0,
    B = 1 << 1,
    C = 1 << 2,
    D = 1 << 3,
    E = 1 << 4,
    H = 1 << 5,
    L = 1 << 6,
    SP = 1 << 7,
    Fc = 1 << 8,
    Fz = 1 << 9,
}

impl From<Reg> for u16 {
    fn from(value: Reg) -> Self {
        value as u16
    }
}

impl TryFrom<u16> for Reg {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value.count_ones() != 1 {
            return Err(());
        }
        Ok(match value.trailing_zeros() {
            0 => Reg::A,
            1 => Reg::B,
            2 => Reg::C,
            3 => Reg::D,
            4 => Reg::E,
            5 => Reg::H,
            6 => Reg::L,
            7 => Reg::SP,
            8 => Reg::Fc,
            9 => Reg::Fz,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Color {
    A = 1 << 0,
    B = 1 << 1,
    C = 1 << 2,
    D = 1 << 3,
    E = 1 << 4,
    H = 1 << 5,
    L = 1 << 6,
    AF = 1 << 7,
    BC = 1 << 8,
    DE = 1 << 9,
    HL = 1 << 10,
    SP = 1 << 11,
    Fz = 1 << 12,
    Fc = 1 << 13,
}

impl TryFrom<u16> for Color {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value.count_ones() != 1 {
            return Err(());
        }
        Ok(match value.trailing_zeros() {
            0 => Color::A,
            1 => Color::B,
            2 => Color::C,
            3 => Color::D,
            4 => Color::E,
            5 => Color::H,
            6 => Color::L,
            7 => Color::AF,
            8 => Color::BC,
            9 => Color::DE,
            10 => Color::HL,
            11 => Color::SP,
            12 => Color::Fz,
            13 => Color::Fc,
            _ => return Err(()),
        })
    }
}

impl Color {
    /// Accumulator
    #[must_use]
    pub const fn acc() -> ColorSet {
        ColorSet::just(Color::A)
    }

    /// general purposes 8-bit registers
    #[must_use]
    pub const fn r8() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::A);
        set.insert(Color::B);
        set.insert(Color::C);
        set.insert(Color::D);
        set.insert(Color::E);
        set.insert(Color::H);
        set.insert(Color::L);
        set
    }

    /// 8-bit register which makes up the low bits of a 16-bit register
    #[must_use]
    pub const fn r8_low() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::C);
        set.insert(Color::E);
        set.insert(Color::L);
        set
    }

    /// 8-bit register which makes up the high bits of a 16-bit register
    #[must_use]
    pub const fn r8_high() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::B);
        set.insert(Color::D);
        set.insert(Color::H);
        set
    }

    /// general purpose 16-bit register
    #[must_use]
    pub const fn r16() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::BC);
        set.insert(Color::DE);
        set.insert(Color::HL);
        set
    }

    /// 16-bit register which you can PUSH and POP from the stack
    #[must_use]
    pub const fn r16_stack() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::BC);
        set.insert(Color::DE);
        set.insert(Color::HL);
        set.insert(Color::AF);
        set
    }

    /// 16-bit register which can be used as a general purpose pointer to memory
    #[must_use]
    pub const fn r16_ptr() -> ColorSet {
        ColorSet::just(Color::HL)
    }

    /// 8-bit register which can be used as a pointer to memory in the `$FFxx` range.
    #[must_use]
    pub const fn r8_high_ptr() -> ColorSet {
        ColorSet::just(Color::C)
    }

    /// 16-bit register which can be used as a pointers to memory during a `LD`
    #[must_use]
    pub const fn r16_ptr_ld() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::BC);
        set.insert(Color::DE);
        set.insert(Color::HL);
        set
    }

    /// flags
    #[must_use]
    pub const fn flags() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::Fz);
        set.insert(Color::Fc);
        set
    }

    /// zero flag
    #[must_use]
    pub const fn z() -> ColorSet {
        ColorSet::just(Color::Fz)
    }

    /// carry flag
    #[must_use]
    pub const fn c() -> ColorSet {
        ColorSet::just(Color::Fc)
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
            Color::BC => {
                let mut set = RegSet::empty();
                set.insert(Reg::B);
                set.insert(Reg::C);
                set
            }
            Color::DE => {
                let mut set = RegSet::empty();
                set.insert(Reg::D);
                set.insert(Reg::E);
                set
            }
            Color::HL => {
                let mut set = RegSet::empty();
                set.insert(Reg::H);
                set.insert(Reg::L);
                set
            }
            Color::AF => {
                let mut set = RegSet::empty();
                set.insert(Reg::A);
                set.insert(Reg::Fz);
                set.insert(Reg::Fc);
                set
            }
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

macro_rules! bit_set_impl {
    ($struct_name:ident; $item_struct_name:ident; $iter_struct_name:ident) => {
        #[derive(Clone, PartialEq, Eq)]
        pub struct $struct_name {
            bits: u16,
        }

        impl $struct_name {
            #[must_use]
            pub const fn empty() -> Self {
                Self { bits: 0 }
            }

            pub const fn just(reg: $item_struct_name) -> Self {
                Self { bits: reg as u16 }
            }
        }

        impl $struct_name {
            #[must_use]
            pub const fn is_empty(&self) -> bool {
                self.bits == 0
            }

            #[must_use]
            pub const fn len(&self) -> usize {
                self.bits.count_ones() as usize
            }

            #[must_use]
            pub const fn contains(&self, value: $item_struct_name) -> bool {
                self.bits & value as u16 != 0
            }

            pub const fn insert(&mut self, value: $item_struct_name) -> bool {
                let bits = value as u16;
                if self.bits & bits != 0 {
                    return false;
                }
                self.bits |= bits;
                true
            }

            pub const fn remove(&mut self, value: $item_struct_name) -> bool {
                let bits = value as u16;
                if self.bits & bits == 0 {
                    return false;
                }
                self.bits &= !bits;
                true
            }

            pub fn retain(&mut self, mut f: impl FnMut($item_struct_name) -> bool) {
                for value in self.clone() {
                    if !f(value) {
                        self.remove(value);
                    }
                }
            }
        }

        impl fmt::Debug for $struct_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if f.alternate() {
                    f.debug_tuple(stringify!($struct_name))
                        .field(&format_args!("{:b}", self.bits))
                        .finish()
                } else {
                    let mut debug_set = f.debug_set();
                    for item in self.clone().into_iter() {
                        debug_set.entry(&item);
                    }
                    debug_set.finish()
                }
            }
        }

        impl BitAnd for $struct_name {
            type Output = Self;

            fn bitand(self, rhs: Self) -> Self::Output {
                Self {
                    bits: self.bits & rhs.bits,
                }
            }
        }

        impl BitAndAssign for $struct_name {
            fn bitand_assign(&mut self, rhs: Self) {
                self.bits &= rhs.bits;
            }
        }

        impl BitOr for $struct_name {
            type Output = Self;

            fn bitor(self, rhs: Self) -> Self::Output {
                Self {
                    bits: self.bits | rhs.bits,
                }
            }
        }

        impl BitOrAssign for $struct_name {
            fn bitor_assign(&mut self, rhs: Self) {
                self.bits |= rhs.bits;
            }
        }

        impl IntoIterator for $struct_name {
            type Item = $item_struct_name;
            type IntoIter = $iter_struct_name;

            fn into_iter(self) -> Self::IntoIter {
                $iter_struct_name(self)
            }
        }

        pub struct $iter_struct_name($struct_name);

        impl Iterator for $iter_struct_name {
            type Item = $item_struct_name;

            fn next(&mut self) -> Option<Self::Item> {
                let value = u16::try_from(self.0.bits.trailing_zeros()).ok()?;
                let value = $item_struct_name::try_from(value).ok()?;
                self.0.remove(value);
                Some(value)
            }
        }
    };
}

bit_set_impl!(RegSet; Reg; RegSetIter);
bit_set_impl!(ColorSet; Color; ColorSetIter);
