#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![allow(clippy::all)]
// SAFETY: It is up to the calling of an unsafe fn in bindings.rs to
// ensure that the unsafe code within fn is in fact safe
#![allow(unsafe_op_in_unsafe_fn)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// SameBoy declares its internal functions with hidden visibility, which keeps them out of the
// generated bindings. The ones needed are declared by hand here.
unsafe extern "C" {
    /// Advances the PPU by `cycles`, and with `force` set also by whatever it is lagging
    /// behind the CPU.
    pub fn GB_display_run(gb: *mut GB_gameboy_t, cycles: ::std::os::raw::c_uint, force: bool);
}

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
