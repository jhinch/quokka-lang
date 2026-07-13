pub mod plumbing;
pub mod porcelain;

pub trait Execute {
    fn execute(self) -> miette::Result<()>;
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

impl Execute for Cli {
    fn execute(self) -> miette::Result<()> {
        self.command.execute()
    }
}

#[derive(clap::Subcommand)]
pub enum CliCommand {
    #[command(flatten)]
    Plumbing(plumbing::PlumbingCommand),
    #[command(flatten)]
    Porcelain(porcelain::PorcelainCommand),
}

impl Execute for CliCommand {
    fn execute(self) -> miette::Result<()> {
        match self {
            CliCommand::Plumbing(command) => command.execute(),
            CliCommand::Porcelain(command) => command.execute(),
        }
    }
}
