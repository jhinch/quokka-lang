use crate::cli::Execute;

pub mod analyze;
pub mod codegen;
pub mod lex;
pub mod parse;

#[derive(clap::Subcommand)]
pub enum PlumbingCommand {
    #[command(hide = true)]
    Lex(lex::LexCommand),
    #[command(hide = true)]
    Parse(parse::ParseCommand),
    #[command(hide = true)]
    Analyze(analyze::AnalyzeCommand),
    #[command(hide = true)]
    Codegen(codegen::CodegenCommand),
}

impl Execute for PlumbingCommand {
    fn execute(self) -> miette::Result<()> {
        match self {
            PlumbingCommand::Lex(command) => command.execute(),
            PlumbingCommand::Parse(command) => command.execute(),
            PlumbingCommand::Analyze(command) => command.execute(),
            PlumbingCommand::Codegen(command) => command.execute(),
        }
    }
}
