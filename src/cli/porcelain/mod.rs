use crate::cli::Execute;

pub mod build;

#[derive(clap::Subcommand)]
pub enum PorcelainCommand {
    Build(build::BuildCommand),
}

impl Execute for PorcelainCommand {
    fn execute(self) -> miette::Result<()> {
        let PorcelainCommand::Build(command) = self;
        command.execute()
    }
}
