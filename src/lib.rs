#![warn(clippy::must_use_candidate)]
#![warn(clippy::unnecessary_semicolon)]
#![warn(clippy::needless_continue)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::return_self_not_must_use)]
pub mod ast;
pub mod feature;
pub mod highlight;
pub mod lex;
pub mod multi;
pub mod parse;
pub mod source;
