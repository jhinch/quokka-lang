#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![allow(clippy::all)]
// SAFETY: It is up to the calling of an unsafe fn in bindings.rs to
// ensure that the unsafe code within fn is in fact safe
#![allow(unsafe_op_in_unsafe_fn)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

pub mod bootroms {
    pub const fn dmg() -> &'static [u8] {
        include_bytes!("../bootroms/dmg_boot.bin")
    }

    pub const fn mgb() -> &'static [u8] {
        include_bytes!("../bootroms/mgb_boot.bin")
    }

    pub const fn cgb() -> &'static [u8] {
        include_bytes!("../bootroms/cgb_boot.bin")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bindings and the C sources are compiled separately, and both need `GB_INTERNAL`
    /// defined to agree on what `GB_gameboy_t` contains.
    #[test]
    fn gameboy_struct_size_matches_c_build() {
        // SAFETY: `GB_allocation_size` has no preconditions
        let c_size = unsafe { GB_allocation_size() };
        assert_eq!(c_size, std::mem::size_of::<GB_gameboy_t>());
    }
}
