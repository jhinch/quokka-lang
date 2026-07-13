use crate::cli::Execute;
use miette::{NamedSource, Report};
use quokka_lang::lex::Lexer;

#[derive(clap::Args)]
pub struct LexCommand {
    filename: String,
}

impl Execute for LexCommand {
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;
        for token in Lexer::from(file_contents.as_str()) {
            let token = token.map_err(|e| {
                Report::new(e)
                    .with_source_code(NamedSource::new(&self.filename, file_contents.clone()))
            })?;
            println!("{:?}:{}", token.kind(), token.text().escape_default());
        }
        Ok(())
    }
}
