use core::fmt;
use std::cmp::Ordering;
use std::num::{IntErrorKind, NonZeroU16};
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
    type Error = TryFromIntError;

    fn try_from(data: u8) -> Result<Self, Self::Error> {
        if data > Self::MAX.data {
            Err(IntErrorKind::PosOverflow.into())
        } else {
            Ok(UInt { data })
        }
    }
}

impl<const BITS: u8> TryFrom<u32> for UInt<BITS> {
    type Error = TryFromIntError;

    fn try_from(data: u32) -> Result<Self, Self::Error> {
        if data > Self::MAX.data as u32 {
            Err(IntErrorKind::PosOverflow.into())
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
    pub const fn bits(self) -> u3 {
        #[allow(clippy::cast_possible_truncation)]
        u3::new(self.data as u8 & u3::MAX.data)
    }

    #[must_use]
    pub const fn padded_bytes(self) -> u16 {
        #[allow(clippy::cast_possible_truncation)]
        let bytes = self.data.div_ceil(8) as u16;
        bytes
    }

    #[must_use]
    pub const fn padded(self) -> BitCount {
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
        debug_assert_eq!(
            data & !Self::MAX.into_u32(),
            0,
            "{:?}",
            IntErrorKind::PosOverflow
        );
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
        debug_assert_eq!(
            data & !Self::MAX.into_u32(),
            0,
            "{:?}",
            IntErrorKind::PosOverflow
        );
        BitCount {
            data: data & BitCount::MAX.data,
        }
    }
}

impl Mul<u32> for BitCount {
    type Output = Self;

    fn mul(self, rhs: u32) -> Self::Output {
        let data = self.into_u32() * rhs;
        debug_assert_eq!(
            data & !Self::MAX.into_u32(),
            0,
            "{:?}",
            IntErrorKind::PosOverflow
        );
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

#[derive(Debug, thiserror::Error)]
#[error("{0:?}")]
pub struct TryFromIntError(IntErrorKind);

impl From<IntErrorKind> for TryFromIntError {
    fn from(value: IntErrorKind) -> Self {
        TryFromIntError(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_and_bits_are_correct_per_width() {
        assert_eq!(u2::MAX.data, 0b11);
        assert_eq!(u2::BITS, 2);
        assert_eq!(u3::MAX.data, 0b111);
        assert_eq!(u3::BITS, 3);
        assert_eq!(u4::MAX.data, 0b1111);
        assert_eq!(u4::BITS, 4);
    }

    #[test]
    fn new_accepts_in_range_values() {
        assert_eq!(u2::new(0).data, 0);
        assert_eq!(u2::new(3).data, 3);
        assert_eq!(u3::new(7).data, 7);
        assert_eq!(u4::new(15).data, 15);
    }

    #[test]
    #[should_panic]
    fn new_panics_when_out_of_range() {
        let _ = u2::new(4);
    }

    #[test]
    #[should_panic]
    fn new_panics_at_max_plus_one_for_u4() {
        let _ = u4::new(16);
    }

    #[test]
    fn try_from_u8_accepts_in_range_and_rejects_out_of_range() {
        assert_eq!(u2::try_from(0u8).unwrap().data, 0);
        assert_eq!(u2::try_from(3u8).unwrap().data, 3);
        assert!(u2::try_from(4u8).is_err());
        assert!(u3::try_from(8u8).is_err());
        assert_eq!(u4::try_from(15u8).unwrap().data, 15);
        assert!(u4::try_from(16u8).is_err());
        assert!(u4::try_from(u8::MAX).is_err());
    }

    #[test]
    fn try_from_u32_accepts_in_range_and_rejects_out_of_range() {
        assert_eq!(u2::try_from(0u32).unwrap().data, 0);
        assert_eq!(u2::try_from(3u32).unwrap().data, 3);
        assert!(u2::try_from(4u32).is_err());
        assert!(u4::try_from(16u32).is_err());
        assert!(u4::try_from(u32::MAX).is_err());
    }

    #[test]
    fn into_u8_and_u32_round_trip_the_stored_value() {
        let value = u4::new(9);
        assert_eq!(u8::from(value), 9u8);
        assert_eq!(u32::from(value), 9u32);
    }

    #[test]
    fn debug_and_display_show_the_underlying_value() {
        let value = u3::new(5);
        assert_eq!(format!("{value:?}"), "5");
        assert_eq!(format!("{value}"), "5");
    }

    #[test]
    fn wrapping_add_wraps_at_bit_width() {
        assert_eq!(u2::new(3).wrapping_add(u2::new(1)), u2::new(0));
        assert_eq!(u2::new(3).wrapping_add(u2::new(2)), u2::new(1));
        assert_eq!(u4::new(15).wrapping_add(u4::new(1)), u4::new(0));
        assert_eq!(u4::new(1).wrapping_add(u4::new(2)), u4::new(3));
    }

    #[test]
    fn wrapping_sub_wraps_at_bit_width() {
        assert_eq!(u2::new(0).wrapping_sub(u2::new(1)), u2::new(3));
        assert_eq!(u4::new(0).wrapping_sub(u4::new(1)), u4::new(15));
        assert_eq!(u4::new(5).wrapping_sub(u4::new(2)), u4::new(3));
    }

    #[test]
    fn to_le_bytes_returns_a_single_byte() {
        assert_eq!(u4::new(9).to_le_bytes(), [9]);
    }

    #[test]
    fn not_complements_within_bit_width_only() {
        assert_eq!(!u2::new(0), u2::new(3));
        assert_eq!(!u2::new(1), u2::new(2));
        assert_eq!(!u4::new(0), u4::new(15));
        assert_eq!(!u4::new(0b1010), u4::new(0b0101));
    }

    #[test]
    fn bitand_bitor_bitxor_operate_bitwise() {
        let a = u4::new(0b1100);
        let b = u4::new(0b1010);
        assert_eq!(a & b, u4::new(0b1000));
        assert_eq!(a | b, u4::new(0b1110));
        assert_eq!(a ^ b, u4::new(0b0110));
    }

    #[test]
    fn shl_and_shr_mask_to_bit_width() {
        assert_eq!(u4::new(0b0011) << 1, u4::new(0b0110));
        assert_eq!(u4::new(0b1000) << 1, u4::new(0b0000));
        assert_eq!(u4::new(0b0110) >> 1, u4::new(0b0011));
        assert_eq!(u4::new(0b0001) >> 1, u4::new(0b0000));
    }

    #[test]
    fn bit_assign_ops_mutate_in_place() {
        let mut value = u4::new(0b1100);
        value &= u4::new(0b1010);
        assert_eq!(value, u4::new(0b1000));

        let mut value = u4::new(0b1100);
        value |= u4::new(0b0011);
        assert_eq!(value, u4::new(0b1111));

        let mut value = u4::new(0b1100);
        value ^= u4::new(0b1010);
        assert_eq!(value, u4::new(0b0110));
    }

    #[test]
    fn add_and_sub_behave_like_normal_arithmetic_within_range() {
        assert_eq!(u4::new(3) + u4::new(4), u4::new(7));
        assert_eq!(u4::new(7) - u4::new(4), u4::new(3));
        assert_eq!(u2::new(1) + u2::new(2), u2::new(3));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn add_panics_on_overflow_in_debug() {
        let _ = u4::new(15) + u4::new(1);
    }

    #[test]
    fn ordering_compares_by_underlying_value() {
        assert!(u4::new(1) < u4::new(2));
        assert!(u4::new(2) <= u4::new(2));
        assert!(u4::new(3) > u4::new(2));
        assert_eq!(u4::new(2), u4::new(2));
        assert_ne!(u4::new(1), u4::new(2));
    }

    #[test]
    fn bit_count_max_and_bits_are_16_plus_3() {
        assert_eq!(BitCount::BITS, u16::BITS + 3);
        assert_eq!(BitCount::MAX.into_u32(), (1u32 << (u16::BITS + 3)) - 1);
    }

    #[test]
    fn bit_count_new_accepts_in_range_values() {
        assert_eq!(BitCount::new(0).into_u32(), 0);
        assert_eq!(BitCount::new(100).into_u32(), 100);
        assert_eq!(
            BitCount::new(BitCount::MAX.into_u32()).into_u32(),
            BitCount::MAX.into_u32()
        );
    }

    #[test]
    #[should_panic]
    fn bit_count_new_panics_when_out_of_range() {
        let _ = BitCount::new(BitCount::MAX.into_u32() + 1);
    }

    #[test]
    fn bit_count_from_u3_carries_the_value() {
        let bc: BitCount = u3::new(5).into();
        assert_eq!(bc.into_u32(), 5);
    }

    #[test]
    fn bit_count_debug_and_display_show_underlying_value() {
        let bc = BitCount::new(42);
        assert_eq!(format!("{bc:?}"), "42");
        assert_eq!(format!("{bc}"), "42");
    }

    #[test]
    fn bit_count_bytes_returns_whole_bytes_rounded_down() {
        assert_eq!(BitCount::new(0).bytes(), 0);
        assert_eq!(BitCount::new(7).bytes(), 0);
        assert_eq!(BitCount::new(8).bytes(), 1);
        assert_eq!(BitCount::new(15).bytes(), 1);
        assert_eq!(BitCount::new(16).bytes(), 2);
    }

    #[test]
    fn bit_count_padding_returns_remainder_bits() {
        assert_eq!(BitCount::new(0).bits(), u3::new(0));
        assert_eq!(BitCount::new(7).bits(), u3::new(7));
        assert_eq!(BitCount::new(8).bits(), u3::new(0));
        assert_eq!(BitCount::new(13).bits(), u3::new(5));
    }

    #[test]
    fn bit_count_padded_bytes_rounds_up_to_whole_bytes() {
        assert_eq!(BitCount::new(0).padded_bytes(), 0);
        assert_eq!(BitCount::new(1).padded_bytes(), 1);
        assert_eq!(BitCount::new(7).padded_bytes(), 1);
        assert_eq!(BitCount::new(8).padded_bytes(), 1);
        assert_eq!(BitCount::new(9).padded_bytes(), 2);
        assert_eq!(BitCount::new(16).padded_bytes(), 2);
        assert_eq!(BitCount::new(17).padded_bytes(), 3);
    }

    #[test]
    fn bit_count_padded_bytes_matches_padded_bit_count_in_bytes() {
        for bits in [0, 1, 7, 8, 9, 15, 16, 17, 100] {
            let bc = BitCount::new(bits);
            assert_eq!(bc.padded_bytes(), bc.padded().bytes());
        }
    }

    #[test]
    fn bit_count_padded_to_whole_byte_rounds_up() {
        assert_eq!(BitCount::new(0).padded().into_u32(), 0);
        assert_eq!(BitCount::new(1).padded().into_u32(), 8);
        assert_eq!(BitCount::new(8).padded().into_u32(), 8);
        assert_eq!(BitCount::new(9).padded().into_u32(), 16);
    }

    #[test]
    fn bit_count_partial_eq_and_ord_against_u32() {
        let bc = BitCount::new(10);
        assert_eq!(bc, 10u32);
        assert_ne!(bc, 11u32);
        assert!(bc < 11u32);
        assert!(bc > 9u32);
        assert!(bc <= 10u32);
        assert!(bc >= 10u32);
    }

    #[test]
    fn bit_count_add_and_sub_with_bit_count_and_u32() {
        assert_eq!((BitCount::new(3) + BitCount::new(4)).into_u32(), 7);
        assert_eq!((BitCount::new(7) - BitCount::new(4)).into_u32(), 3);
        assert_eq!((BitCount::new(3) + 4u32).into_u32(), 7);
        assert_eq!((BitCount::new(7) - 4u32).into_u32(), 3);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn bit_count_add_panics_on_overflow_in_debug() {
        let _ = BitCount::MAX + 1u32;
    }

    #[test]
    fn bit_count_mul_by_nonzero_u16_scales_the_value() {
        let rhs = NonZeroU16::new(3).unwrap();
        assert_eq!((BitCount::new(5) * rhs).into_u32(), 15);
        let one = NonZeroU16::new(1).unwrap();
        assert_eq!((BitCount::new(5) * one).into_u32(), 5);
    }

    #[test]
    fn bit_count_mul_by_u32_scales_the_value_without_overflow() {
        assert_eq!((BitCount::new(5) * 3u32).into_u32(), 15);
    }

    #[test]
    fn bit_count_add_assign_and_sub_assign_mutate_in_place() {
        let mut bc = BitCount::new(3);
        bc += BitCount::new(4);
        assert_eq!(bc.into_u32(), 7);
        bc -= BitCount::new(2);
        assert_eq!(bc.into_u32(), 5);
    }
}
