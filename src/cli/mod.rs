pub mod plumbing;
pub mod porcelain;

pub trait Execute {
    fn execute(self) -> miette::Result<()>;
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,

    /// Increase logging verbosity (-v for debug, -vv for trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Cli {
    pub fn tracing_level(&self) -> tracing::Level {
        match self.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    }
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
