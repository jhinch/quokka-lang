use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::pipeline::PipelineConfig;
use tracing::instrument;

#[derive(clap::Args)]
pub struct ParseCommand {
    filename: String,
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for ParseCommand {
    #[instrument(skip_all)]
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        let output = PipelineConfig::default()
            .with_features(self.preview_features.iter().cloned().collect())
            .start()
            .parse(&file_contents)
            .map_err(|e| {
                miette::Report::new(e)
                    .with_source_code(miette::NamedSource::new(&self.filename, file_contents))
            })?;
        println!(
            "{}",
            ron::ser::to_string_pretty(
                &output.ast(),
                ron::ser::PrettyConfig::new()
                    .struct_names(true)
                    .indentor("  ")
            )
            .map_err(miette::Report::from_err)?,
        );

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
        cmd: ParseCommand,
    }

    fn parse(args: &[&str]) -> Result<ParseCommand, clap::Error> {
        let mut full = vec!["cmd"];
        full.extend_from_slice(args);
        TestCli::try_parse_from(full).map(|cli| cli.cmd)
    }

    #[test]
    fn parses_filename_only() {
        let cmd = parse(&["file.q"]).unwrap();
        assert_eq!(cmd.filename, "file.q");
        assert!(cmd.preview_features.is_empty());
    }

    #[test]
    fn missing_filename_is_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn parses_multiple_preview_features() {
        let cmd = parse(&["-p", "example-preview", "-p", "narrow-int", "file.q"]).unwrap();
        assert_eq!(
            cmd.preview_features,
            vec![PreviewFeature::ExamplePreview, PreviewFeature::NarrowInt]
        );
    }

    #[test]
    fn unknown_preview_feature_is_error() {
        assert!(parse(&["--preview", "bogus", "file.q"]).is_err());
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse(&["--not-a-flag", "file.q"]).is_err());
    }
}
