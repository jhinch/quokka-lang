use core::fmt;
use std::cmp::Ordering;
use std::num::NonZeroU16;
use std::ops::{
    Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Mul, Not, Shl,
    Shr, Sub, SubAssign,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct UInt<const BITS: u8> {
    data: u8,
}

#[allow(non_camel_case_types)]
pub type u2 = UInt<2>;

#[allow(non_camel_case_types)]
pub type u3 = UInt<3>;
#[allow(non_camel_case_types)]
pub type u4 = UInt<4>;

impl<const BITS: u8> UInt<BITS> {
    const MAX: Self = UInt {
        data: u8::MAX >> (8 - BITS),
    };
    const BITS: u32 = Self::MAX.data.count_ones();

    #[must_use]
    pub const fn new(data: u8) -> Self {
        assert!(data <= Self::MAX.data);
        UInt { data }
    }
}

impl<const BITS: u8> TryFrom<u8> for UInt<BITS> {
    type Error = ();

    fn try_from(data: u8) -> Result<Self, Self::Error> {
        if data > Self::MAX.data {
            Err(())
        } else {
            Ok(UInt { data })
        }
    }
}

impl<const BITS: u8> TryFrom<u32> for UInt<BITS> {
    type Error = ();

    fn try_from(data: u32) -> Result<Self, Self::Error> {
        if data > Self::MAX.data as u32 {
            Err(())
        } else {
            #[allow(clippy::cast_possible_truncation)]
            Ok(UInt { data: data as u8 })
        }
    }
}

impl<const BITS: u8> From<UInt<BITS>> for u8 {
    fn from(value: UInt<BITS>) -> Self {
        value.data
    }
}

impl<const BITS: u8> From<UInt<BITS>> for u32 {
    fn from(value: UInt<BITS>) -> Self {
        u32::from(value.data)
    }
}

impl<const BITS: u8> fmt::Debug for UInt<BITS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.data, f)
    }
}

impl<const BITS: u8> fmt::Display for UInt<BITS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.data, f)
    }
}

impl<const BITS: u8> UInt<BITS> {
    #[must_use]
    pub const fn wrapping_add(self, other: Self) -> Self {
        UInt {
            data: self.data.wrapping_add(other.data) & Self::MAX.data,
        }
    }

    #[must_use]
    pub const fn wrapping_sub(self, other: Self) -> Self {
        UInt {
            data: self.data.wrapping_sub(other.data) & Self::MAX.data,
        }
    }

    #[must_use]
    pub const fn to_le_bytes(self) -> [u8; 1] {
        [self.data]
    }
}

impl<const BITS: u8> Not for UInt<BITS> {
    type Output = Self;

    fn not(self) -> Self::Output {
        let data = !self.data;
        UInt {
            data: data & Self::MAX.data,
        }
    }
}

impl<const BITS: u8> BitAnd for UInt<BITS> {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        let data = self.data & rhs.data;
        UInt { data }
    }
}

impl<const BITS: u8> BitOr for UInt<BITS> {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        let data = self.data | rhs.data;
        UInt { data }
    }
}

impl<const BITS: u8> BitXor for UInt<BITS> {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        let data = self.data ^ rhs.data;
        UInt { data }
    }
}

impl<const BITS: u8> Shl<u8> for UInt<BITS> {
    type Output = Self;

    fn shl(self, rhs: u8) -> Self::Output {
        let data = self.data << rhs;
        UInt {
            data: data & Self::MAX.data,
        }
    }
}

impl<const BITS: u8> Shr<u8> for UInt<BITS> {
    type Output = Self;

    fn shr(self, rhs: u8) -> Self::Output {
        let data = self.data >> rhs;
        UInt {
            data: data & Self::MAX.data,
        }
    }
}

impl<const BITS: u8> BitAndAssign for UInt<BITS> {
    fn bitand_assign(&mut self, rhs: Self) {
        self.data &= rhs.data;
    }
}

impl<const BITS: u8> BitOrAssign for UInt<BITS> {
    fn bitor_assign(&mut self, rhs: Self) {
        self.data |= rhs.data;
    }
}

impl<const BITS: u8> BitXorAssign for UInt<BITS> {
    fn bitxor_assign(&mut self, rhs: Self) {
        self.data ^= rhs.data;
    }
}

impl<const BITS: u8> Add for UInt<BITS> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        let data = self.data + rhs.data;
        debug_assert_eq!(data & !Self::MAX.data, 0, "overflow");
        UInt {
            data: data & Self::MAX.data,
        }
    }
}

impl<const BITS: u8> Sub for UInt<BITS> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        let data = self.data - rhs.data;
        UInt {
            data: data & Self::MAX.data,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitCount {
    data: u32,
}

impl BitCount {
    pub const BITS: u32 = u16::BITS + u3::BITS;
    pub const MAX: BitCount = BitCount {
        data: u32::MAX >> (u32::BITS - Self::BITS),
    };

    #[must_use]
    pub const fn new(data: u32) -> Self {
        assert!(data <= Self::MAX.into_u32());
        BitCount { data }
    }
}

impl From<u3> for BitCount {
    fn from(data: u3) -> Self {
        BitCount {
            data: u32::from(data.data),
        }
    }
}

impl From<BitCount> for u32 {
    fn from(value: BitCount) -> Self {
        value.data
    }
}

impl fmt::Debug for BitCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.data, f)
    }
}

impl fmt::Display for BitCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.data, f)
    }
}

impl BitCount {
    #[must_use]
    pub const fn into_u32(self) -> u32 {
        self.data
    }

    #[must_use]
    pub const fn bytes(self) -> u16 {
        #[allow(clippy::cast_possible_truncation)]
        let bytes = (self.data >> u3::BITS) as u16;
        bytes
    }

    #[must_use]
    pub const fn padding(self) -> u3 {
        #[allow(clippy::cast_possible_truncation)]
        u3::new(self.data as u8 & u3::MAX.data)
    }

    #[must_use]
    pub const fn padded_to_whole_byte(self) -> BitCount {
        BitCount::new(self.data.next_multiple_of(8))
    }
}

impl PartialEq<u32> for BitCount {
    fn eq(&self, other: &u32) -> bool {
        PartialEq::eq(&self.data, other)
    }
}

impl PartialOrd<u32> for BitCount {
    fn partial_cmp(&self, other: &u32) -> Option<Ordering> {
        Some(Ord::cmp(&self.data, other))
    }
}

impl Add for BitCount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        self + rhs.into_u32()
    }
}

impl Sub for BitCount {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self - rhs.into_u32()
    }
}

impl Add<u32> for BitCount {
    type Output = Self;

    fn add(self, rhs: u32) -> Self::Output {
        let data = self.into_u32() + rhs;
        debug_assert_eq!(data & !Self::MAX.into_u32(), 0, "overflow");
        BitCount {
            data: data & BitCount::MAX.data,
        }
    }
}

impl Sub<u32> for BitCount {
    type Output = Self;

    fn sub(self, rhs: u32) -> Self::Output {
        let data = self.into_u32() - rhs;
        BitCount {
            data: data & BitCount::MAX.data,
        }
    }
}

impl Mul<NonZeroU16> for BitCount {
    type Output = Self;

    fn mul(self, rhs: NonZeroU16) -> Self::Output {
        let data = self.into_u32() * u32::from(rhs.get());
        debug_assert_eq!(data & !Self::MAX.into_u32(), 0, "overflow");
        BitCount {
            data: data & BitCount::MAX.data,
        }
    }
}

impl Mul<u32> for BitCount {
    type Output = Self;

    fn mul(self, rhs: u32) -> Self::Output {
        let data = self.into_u32() * rhs;
        debug_assert_ne!(data & !Self::MAX.into_u32(), 0, "overflow");
        BitCount {
            data: data & BitCount::MAX.data,
        }
    }
}

impl AddAssign for BitCount {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for BitCount {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}
