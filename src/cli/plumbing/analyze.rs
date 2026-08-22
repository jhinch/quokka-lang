use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::pipeline::PipelineConfig;
use tracing::instrument;

#[derive(clap::Args)]
pub struct AnalyzeCommand {
    filename: String,
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for AnalyzeCommand {
    #[instrument(skip(self))]
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        self.analyze(&file_contents).map_err(|report| {
            report.with_source_code(miette::NamedSource::new(&self.filename, file_contents))
        })?;
        Ok(())
    }
}

impl AnalyzeCommand {
    fn analyze(&self, file_contents: &str) -> miette::Result<()> {
        let output = PipelineConfig::default()
            .with_features(self.preview_features.iter().cloned().collect())
            .start()
            .parse(file_contents)?
            .analyze()?;
        println!("{}", output.mir().display(output.ty_registry()));
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
        cmd: AnalyzeCommand,
    }

    fn analyze(args: &[&str]) -> Result<AnalyzeCommand, clap::Error> {
        let mut full = vec!["cmd"];
        full.extend_from_slice(args);
        TestCli::try_parse_from(full).map(|cli| cli.cmd)
    }

    #[test]
    fn parses_filename_only() {
        let cmd = analyze(&["file.q"]).unwrap();
        assert_eq!(cmd.filename, "file.q");
        assert!(cmd.preview_features.is_empty());
    }

    #[test]
    fn missing_filename_is_error() {
        assert!(analyze(&[]).is_err());
    }

    #[test]
    fn parses_multiple_preview_features() {
        let cmd = analyze(&["-p", "array", "-p", "if-else", "file.q"]).unwrap();
        assert_eq!(
            cmd.preview_features,
            vec![PreviewFeature::Array, PreviewFeature::IfElse]
        );
    }

    #[test]
    fn unknown_preview_feature_is_error() {
        assert!(analyze(&["--preview", "bogus", "file.q"]).is_err());
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(analyze(&["--not-a-flag", "file.q"]).is_err());
    }
}
