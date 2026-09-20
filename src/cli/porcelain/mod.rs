use crate::cli::Execute;

pub mod build;
pub mod lsp;

#[derive(clap::Subcommand)]
pub enum PorcelainCommand {
    Build(build::BuildCommand),
    Lsp(lsp::LspCommand),
}

impl Execute for PorcelainCommand {
    fn execute(self) -> miette::Result<()> {
        match self {
            PorcelainCommand::Build(command) => command.execute(),
            PorcelainCommand::Lsp(command) => command.execute(),
        }
    }
}
