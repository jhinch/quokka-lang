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
    // the order dictates color selection priority order
    A = 1 << 0,
    F = 1 << 1,
    B = 1 << 2,
    C = 1 << 3,
    D = 1 << 4,
    E = 1 << 5,
    H = 1 << 6,
    L = 1 << 7,
    AF = 1 << 8,
    BC = 1 << 9,
    DE = 1 << 10,
    HL = 1 << 11,
    SP = 1 << 12,
    Fz = 1 << 13,
    Fc = 1 << 14,
}

impl From<Color> for u16 {
    fn from(value: Color) -> Self {
        value as u16
    }
}

impl TryFrom<u16> for Color {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value.count_ones() != 1 {
            return Err(());
        }
        Ok(match value.trailing_zeros() {
            0 => Color::A,
            1 => Color::F,
            2 => Color::B,
            3 => Color::C,
            4 => Color::D,
            5 => Color::E,
            6 => Color::H,
            7 => Color::L,
            8 => Color::AF,
            9 => Color::BC,
            10 => Color::DE,
            11 => Color::HL,
            12 => Color::SP,
            13 => Color::Fz,
            14 => Color::Fc,
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
        set.insert(Color::F);
        set.insert(Color::C);
        set.insert(Color::E);
        set.insert(Color::L);
        set
    }

    /// 8-bit register which makes up the high bits of a 16-bit register
    #[must_use]
    pub const fn r8_high() -> ColorSet {
        let mut set = ColorSet::empty();
        set.insert(Color::A);
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
            Color::F => {
                let mut set = RegSet::empty();
                set.insert(Reg::Fz);
                set.insert(Reg::Fc);
                set
            }
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

        impl FromIterator<$item_struct_name> for $struct_name {
            fn from_iter<T: IntoIterator<Item = $item_struct_name>>(iter: T) -> Self {
                let mut set = Self::empty();
                for item in iter {
                    set.insert(item);
                }
                set
            }
        }

        pub struct $iter_struct_name($struct_name);

        impl Iterator for $iter_struct_name {
            type Item = $item_struct_name;

            fn next(&mut self) -> Option<Self::Item> {
                let value = u16::try_from(1 << self.0.bits.trailing_zeros()).ok()?;
                let value = $item_struct_name::try_from(value).ok()?;
                self.0.remove(value);
                Some(value)
            }
        }
    };
}

bit_set_impl!(RegSet; Reg; RegSetIter);
bit_set_impl!(ColorSet; Color; ColorSetIter);

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_REGS: [Reg; 10] = [
        Reg::A,
        Reg::B,
        Reg::C,
        Reg::D,
        Reg::E,
        Reg::H,
        Reg::L,
        Reg::SP,
        Reg::Fc,
        Reg::Fz,
    ];

    const ALL_COLORS: [Color; 15] = [
        Color::A,
        Color::F,
        Color::B,
        Color::C,
        Color::D,
        Color::E,
        Color::H,
        Color::L,
        Color::AF,
        Color::BC,
        Color::DE,
        Color::HL,
        Color::SP,
        Color::Fz,
        Color::Fc,
    ];

    #[test]
    fn reg_to_u16_is_a_single_bit() {
        for reg in ALL_REGS {
            assert_eq!(u16::from(reg).count_ones(), 1, "{reg:?}");
        }
        assert_eq!(u16::from(Reg::A), 1);
        assert_eq!(u16::from(Reg::SP), 1 << 7);
        assert_eq!(u16::from(Reg::Fz), 1 << 9);
    }

    #[test]
    fn reg_try_from_u16_round_trips() {
        for reg in ALL_REGS {
            assert_eq!(Reg::try_from(u16::from(reg)), Ok(reg), "{reg:?}");
        }
    }

    #[test]
    fn reg_try_from_u16_rejects_non_single_bit() {
        assert_eq!(Reg::try_from(0), Err(()));
        assert_eq!(Reg::try_from(0b11), Err(()));
        assert_eq!(Reg::try_from(u16::MAX), Err(()));
    }

    #[test]
    fn reg_try_from_u16_rejects_bit_outside_range() {
        assert_eq!(Reg::try_from(1 << 10), Err(()));
        assert_eq!(Reg::try_from(1 << 15), Err(()));
    }

    #[test]
    fn color_try_from_u16_round_trips() {
        for color in ALL_COLORS {
            assert_eq!(Color::try_from(u16::from(color)), Ok(color), "{color:?}");
        }
    }

    #[test]
    fn color_try_from_u16_rejects_invalid() {
        assert_eq!(Color::try_from(0), Err(()));
        assert_eq!(Color::try_from(0b101), Err(()));
        assert_eq!(Color::try_from(1 << 15), Err(()));
    }

    #[test]
    fn color_regs_single_register_mappings() {
        assert_eq!(Color::A.regs(), RegSet::just(Reg::A));
        assert_eq!(Color::B.regs(), RegSet::just(Reg::B));
        assert_eq!(Color::L.regs(), RegSet::just(Reg::L));
        assert_eq!(Color::SP.regs(), RegSet::just(Reg::SP));
        assert_eq!(Color::Fz.regs(), RegSet::just(Reg::Fz));
        assert_eq!(Color::Fc.regs(), RegSet::just(Reg::Fc));
    }

    #[test]
    fn color_regs_composite_register_mappings() {
        let f = Color::F.regs();
        assert_eq!(f.len(), 2);
        assert!(f.contains(Reg::Fz) && f.contains(Reg::Fc));

        let bc = Color::BC.regs();
        assert_eq!(bc.len(), 2);
        assert!(bc.contains(Reg::B) && bc.contains(Reg::C));

        let hl = Color::HL.regs();
        assert_eq!(hl.len(), 2);
        assert!(hl.contains(Reg::H) && hl.contains(Reg::L));

        let af = Color::AF.regs();
        assert_eq!(af.len(), 3);
        assert!(af.contains(Reg::A) && af.contains(Reg::Fz) && af.contains(Reg::Fc));
    }

    #[test]
    fn color_conflicts_when_registers_overlap() {
        assert!(Color::A.conflicts(Color::A));
        assert!(Color::A.conflicts(Color::AF));
        assert!(Color::B.conflicts(Color::BC));
        assert!(Color::C.conflicts(Color::BC));
        assert!(Color::F.conflicts(Color::Fz));
        assert!(Color::F.conflicts(Color::AF));
        assert!(Color::HL.conflicts(Color::L));
    }

    #[test]
    fn color_does_not_conflict_when_registers_are_disjoint() {
        assert!(!Color::A.conflicts(Color::B));
        assert!(!Color::A.conflicts(Color::F));
        assert!(!Color::BC.conflicts(Color::DE));
        assert!(!Color::HL.conflicts(Color::SP));
        assert!(!Color::Fz.conflicts(Color::Fc));
    }

    #[test]
    fn color_conflicts_is_symmetric() {
        for a in ALL_COLORS {
            for b in ALL_COLORS {
                assert_eq!(a.conflicts(b), b.conflicts(a), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn color_group_helpers_have_expected_members() {
        assert_eq!(Color::acc(), ColorSet::just(Color::A));
        assert_eq!(Color::r16_ptr(), ColorSet::just(Color::HL));
        assert_eq!(Color::r8_high_ptr(), ColorSet::just(Color::C));
        assert_eq!(Color::z(), ColorSet::just(Color::Fz));
        assert_eq!(Color::c(), ColorSet::just(Color::Fc));

        let expect = |set: ColorSet, members: &[Color]| {
            assert_eq!(set.len(), members.len());
            for &m in members {
                assert!(set.contains(m), "{m:?}");
            }
        };

        expect(
            Color::r8(),
            &[
                Color::A,
                Color::B,
                Color::C,
                Color::D,
                Color::E,
                Color::H,
                Color::L,
            ],
        );
        expect(Color::r8_low(), &[Color::F, Color::C, Color::E, Color::L]);
        expect(Color::r8_high(), &[Color::A, Color::B, Color::D, Color::H]);
        expect(Color::r16(), &[Color::BC, Color::DE, Color::HL]);
        expect(
            Color::r16_stack(),
            &[Color::BC, Color::DE, Color::HL, Color::AF],
        );
        expect(Color::r16_ptr_ld(), &[Color::BC, Color::DE, Color::HL]);
        expect(Color::flags(), &[Color::Fz, Color::Fc]);
    }

    #[test]
    fn set_empty_has_no_members() {
        let set = RegSet::empty();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.into_iter().count(), 0);
    }

    #[test]
    fn set_just_holds_a_single_member() {
        let set = RegSet::just(Reg::D);
        assert!(!set.is_empty());
        assert_eq!(set.len(), 1);
        assert!(set.contains(Reg::D));
        assert!(!set.contains(Reg::E));
        assert_eq!(set.into_iter().collect::<Vec<_>>(), vec![Reg::D]);
    }

    #[test]
    fn set_insert_reports_whether_member_was_new() {
        let mut set = ColorSet::empty();
        assert!(set.insert(Color::B));
        assert!(!set.insert(Color::B));
        assert!(set.insert(Color::C));
        assert_eq!(set.len(), 2);
        assert!(set.contains(Color::B) && set.contains(Color::C));
    }

    #[test]
    fn set_remove_reports_whether_member_was_present() {
        let mut set = ColorSet::empty();
        set.insert(Color::B);
        assert!(set.remove(Color::B));
        assert!(!set.remove(Color::B));
        assert!(!set.remove(Color::C));
        assert!(set.is_empty());
    }

    #[test]
    fn set_retain_keeps_only_matching_members() {
        let mut set = RegSet::empty();
        for reg in ALL_REGS {
            set.insert(reg);
        }
        set.retain(|reg| matches!(reg, Reg::A | Reg::B | Reg::C));
        assert_eq!(set.len(), 3);
        assert!(set.contains(Reg::A) && set.contains(Reg::B) && set.contains(Reg::C));
        assert!(!set.contains(Reg::D));
    }

    #[test]
    fn set_bitand_is_intersection() {
        let mut a = RegSet::empty();
        a.insert(Reg::A);
        a.insert(Reg::B);
        a.insert(Reg::C);
        let mut b = RegSet::empty();
        b.insert(Reg::B);
        b.insert(Reg::C);
        b.insert(Reg::D);

        let intersection = a.clone() & b.clone();
        assert_eq!(intersection.len(), 2);
        assert!(intersection.contains(Reg::B) && intersection.contains(Reg::C));

        let mut assigned = a;
        assigned &= b;
        assert_eq!(assigned, intersection);
    }

    #[test]
    fn set_bitor_is_union() {
        let mut a = RegSet::empty();
        a.insert(Reg::A);
        a.insert(Reg::B);
        let mut b = RegSet::empty();
        b.insert(Reg::B);
        b.insert(Reg::C);

        let union = a.clone() | b.clone();
        assert_eq!(union.len(), 3);
        assert!(union.contains(Reg::A) && union.contains(Reg::B) && union.contains(Reg::C));

        let mut assigned = a;
        assigned |= b;
        assert_eq!(assigned, union);
    }

    #[test]
    fn set_iterates_members_in_ascending_bit_order() {
        let mut set = ColorSet::empty();
        set.insert(Color::HL);
        set.insert(Color::A);
        set.insert(Color::B);
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            vec![Color::A, Color::B, Color::HL]
        );
    }

    #[test]
    fn set_iterates_over_every_member() {
        let mut set = RegSet::empty();
        for reg in ALL_REGS {
            set.insert(reg);
        }
        let mut items = set.into_iter().collect::<Vec<_>>();
        items.sort_by_key(|r| u16::from(*r));
        let mut expected = ALL_REGS.to_vec();
        expected.sort_by_key(|r| u16::from(*r));
        assert_eq!(items, expected);
    }

    #[test]
    fn set_debug_lists_members() {
        let mut set = RegSet::empty();
        set.insert(Reg::A);
        set.insert(Reg::B);
        assert_eq!(format!("{set:?}"), "{A, B}");
        assert_eq!(format!("{:?}", RegSet::empty()), "{}");
    }

    #[test]
    fn set_debug_alternate_shows_raw_bits() {
        let mut set = RegSet::empty();
        set.insert(Reg::A);
        set.insert(Reg::C);
        let rendered = format!("{set:#?}");
        assert!(rendered.contains("RegSet"), "{rendered}");
        assert!(rendered.contains("101"), "{rendered}");
    }
}
