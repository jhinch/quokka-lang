use crate::cli::Execute;
use quokka_lang::analyze::Analyzer;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::parse::Parser;
use tracing::instrument;

#[derive(clap::Args)]
pub struct CodegenCommand {
    filename: String,
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for CodegenCommand {
    #[instrument(skip(self))]
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        self.codegen(&file_contents).map_err(|report| {
            report.with_source_code(miette::NamedSource::new(&self.filename, file_contents))
        })?;
        Ok(())
    }
}

impl CodegenCommand {
    fn codegen(&self, file_contents: &str) -> miette::Result<()> {
        let mut parser = Parser::from(file_contents);
        parser.enable_features(self.preview_features.iter().cloned().collect());
        let ast = parser.parse().into_result().map_err(miette::Report::new)?;
        let (mir, ty_registry) = Analyzer::default()
            .analyze(&ast)
            .map_err(miette::Report::new)?;
        let lir = quokka_lang::codegen::CodeGen::from(ty_registry)
            .generate(&mir)
            .map_err(miette::Report::new)?;
        println!("{lir}");
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
        cmd: CodegenCommand,
    }

    fn codegen(args: &[&str]) -> Result<CodegenCommand, clap::Error> {
        let mut full = vec!["cmd"];
        full.extend_from_slice(args);
        TestCli::try_parse_from(full).map(|cli| cli.cmd)
    }

    #[test]
    fn parses_filename_only() {
        let cmd = codegen(&["file.q"]).unwrap();
        assert_eq!(cmd.filename, "file.q");
        assert!(cmd.preview_features.is_empty());
    }

    #[test]
    fn missing_filename_is_error() {
        assert!(codegen(&[]).is_err());
    }

    #[test]
    fn parses_multiple_preview_features() {
        let cmd = codegen(&["-p", "array", "-p", "if-else", "file.q"]).unwrap();
        assert_eq!(
            cmd.preview_features,
            vec![PreviewFeature::Array, PreviewFeature::IfElse]
        );
    }

    #[test]
    fn unknown_preview_feature_is_error() {
        assert!(codegen(&["--preview", "bogus", "file.q"]).is_err());
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(codegen(&["--not-a-flag", "file.q"]).is_err());
    }
}
