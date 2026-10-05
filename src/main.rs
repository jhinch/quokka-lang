#![deny(unsafe_code)]
#![deny(clippy::unnecessary_semicolon)]
#![deny(clippy::needless_continue)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(clippy::must_use_candidate)]
#![deny(clippy::return_self_not_must_use)]
#![deny(clippy::cast_possible_truncation)]
#![deny(clippy::manual_assert)]

use crate::cli::Execute;
use quokka_lang::highlight::QuokkaHighlighter;

mod cli;

fn main() -> miette::Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .with_syntax_highlighting(QuokkaHighlighter)
                .build(),
        )
    }))?;
    let cli = <cli::Cli as clap::Parser>::parse();

    tracing_subscriber::fmt()
        .with_max_level(cli.tracing_level())
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    cli.execute()
}
