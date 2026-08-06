#![deny(clippy::unnecessary_semicolon)]
#![deny(clippy::needless_continue)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(clippy::must_use_candidate)]
#![deny(clippy::return_self_not_must_use)]
pub mod analyze;
pub mod ast;
pub mod attr;
pub mod feature;
pub mod highlight;
pub mod lex;
pub mod lir;
pub mod mir;
pub mod multi;
pub mod parse;
pub mod source;
pub mod ty;
