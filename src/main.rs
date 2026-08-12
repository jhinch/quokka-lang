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
