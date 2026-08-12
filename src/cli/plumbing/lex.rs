use crate::cli::Execute;
use miette::{NamedSource, Report};
use quokka_lang::lex::Lexer;
use tracing::instrument;

#[derive(clap::Args)]
pub struct LexCommand {
    filename: String,
}

impl Execute for LexCommand {
    #[instrument(skip(self))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        cmd: LexCommand,
    }

    fn parse(args: &[&str]) -> Result<LexCommand, clap::Error> {
        let mut full = vec!["cmd"];
        full.extend_from_slice(args);
        TestCli::try_parse_from(full).map(|cli| cli.cmd)
    }

    #[test]
    fn parses_filename() {
        let cmd = parse(&["file.q"]).unwrap();
        assert_eq!(cmd.filename, "file.q");
    }

    #[test]
    fn missing_filename_is_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn too_many_positional_args_is_error() {
        assert!(parse(&["file.q", "extra.q"]).is_err());
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse(&["--not-a-flag", "file.q"]).is_err());
    }
}
