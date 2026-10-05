#![deny(unsafe_code)]
#![deny(clippy::unnecessary_semicolon)]
#![deny(clippy::needless_continue)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(clippy::must_use_candidate)]
#![deny(clippy::return_self_not_must_use)]
#![deny(clippy::cast_possible_truncation)]
#![deny(clippy::manual_assert)]

pub mod dense;
pub mod sparse;
pub mod stable_sparse;

pub use dense::SlotKeySet;
pub use sparse::SparseSlotKeySet;
pub use stable_sparse::StableSparseSlotKeySet;
