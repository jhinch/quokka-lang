#![deny(clippy::unnecessary_semicolon)]
#![deny(clippy::needless_continue)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(clippy::must_use_candidate)]
#![deny(clippy::return_self_not_must_use)]
#![deny(clippy::cast_possible_truncation)]
#![deny(clippy::manual_assert)]
#![deny(clippy::undocumented_unsafe_blocks)]

use quokka_lang_sameboy_sys::*;
use std::pin::Pin;

pub struct Sameboy {
    inner: Pin<Box<GB_gameboy_t>>,
    pixels: (*mut u32, usize),
}

impl Sameboy {
    #[must_use]
    pub fn new(model: Model) -> Self {
        let model = model as GB_model_t;
        let mut inner = Box::<GB_gameboy_t>::new_uninit();
        // SAFETY: `GB_init` requires memory is `sizeof(GB_gameboy_t)` which
        // `Box::<GB_gameboy_t>::new_uninit()` guarantees
        unsafe {
            GB_init(inner.as_mut_ptr(), model);
        }
        // SAFETY: GB_init guarantees to initialize the memory
        let inner = unsafe { inner.assume_init() };
        let mut inner = Box::into_pin(inner);

        // SAFETY: `GB_get_screen_width` and `GB_get_screen_height` require `*mut GB_gameboy_t`
        // to point to initialized memory which Box<_> guarantees
        let (screen_width, screen_height): (u32, u32) = unsafe {
            (
                GB_get_screen_width(inner.as_mut().get_mut()),
                GB_get_screen_height(inner.as_mut().get_mut()),
            )
        };
        let size = (screen_height as usize)
            .checked_mul(screen_width as usize)
            .expect("Screen too large");
        let mut pixels = vec![0u32; size];
        // this shouldn't be needed, but guarantees capacity() == len()
        pixels.shrink_to_fit();
        // leaks the vec, but we will reconstruct it within Drop
        let (pixels_ptr, len, capacity) = pixels.into_raw_parts();
        debug_assert_eq!(len, capacity);
        // SAFETY: `GB_set_pixels_output` requires `*mut GB_gameboy_t` to point to initialized
        // memory and `*mut 32` to point to a buffer of size `height * width` which Box<_> and Vec<u8>
        // guarantee respectively. pixels_ptr has only just been leaked so it is valid.
        unsafe {
            GB_set_pixels_output(inner.as_mut().get_mut(), pixels_ptr);
        }

        Sameboy {
            inner,
            pixels: (pixels_ptr, len),
        }
    }

    pub fn reset(&mut self) {
        // SAFETY: `GB_reset` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_reset(self.inner.as_mut().get_mut());
        }
    }

    pub fn load_rom(&mut self, rom: &[u8]) {
        // SAFETY: `GB_load_rom_from_buffer` requires `*mut GB_gameboy_t` to point to initialized
        // memory and `*const u8` to point to a buffer of size `size` which Box<_> and &[u8]
        // guarantee respectively
        unsafe {
            GB_load_rom_from_buffer(self.inner.as_mut().get_mut(), rom.as_ptr(), rom.len());
        }
    }

    /// Run one CPU instruction. Returns the time passed in 8MHz ticks.
    ///
    /// For example, if the next instruction is NOP, this function returns 8 if in normal
    /// mode and 4 in double speed (GBC) mode.
    pub fn run(&mut self) -> u32 {
        // SAFETY: `GB_run` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe { GB_run(self.inner.as_mut().get_mut()) }
    }
}

impl Drop for Sameboy {
    fn drop(&mut self) {
        // SAFETY: `GB_set_pixels_output` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_set_pixels_output(self.inner.as_mut().get_mut(), std::ptr::null_mut());
        }

        // SAFETY: We now have exclusive access to the pixel pointer as we nulled out the
        // reference passed to sameboy. We now can safely drop the memory
        drop(unsafe { Vec::from_raw_parts(self.pixels.0, self.pixels.1, self.pixels.1) });

        // SAFETY: `GB_free` requires `*mut GB_gameboy_t` to point to initialized
        // memory which Box<_> guarantees
        unsafe {
            GB_free(self.inner.as_mut().get_mut());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum Model {
    Dmg = GB_model_t_GB_MODEL_DMG_B,
}
