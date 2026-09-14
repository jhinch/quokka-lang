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
