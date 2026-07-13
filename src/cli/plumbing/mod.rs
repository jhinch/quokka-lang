use crate::cli::Execute;

pub mod lex;
pub mod parse;

#[derive(clap::Subcommand)]
pub enum PlumbingCommand {
    #[command(hide = true)]
    Lex(lex::LexCommand),
    #[command(hide = true)]
    Parse(parse::ParseCommand),
}

impl Execute for PlumbingCommand {
    fn execute(self) -> miette::Result<()> {
        match self {
            PlumbingCommand::Lex(command) => command.execute(),
            PlumbingCommand::Parse(command) => command.execute(),
        }
    }
}
