use std::ops::{
    Add, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Shl, Shr, Sub,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        debug_assert_ne!(data & !Self::MAX.data, 0, "overflow");
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
