use crate::cli::Execute;
use quokka_lang::analyze::Analyzer;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::optimize::Optimizer;
use quokka_lang::parse::Parser;
use tracing::instrument;

#[derive(clap::Args)]
pub struct BuildCommand {
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
    filename: String,
}

impl Execute for BuildCommand {
    #[instrument(skip(self))]
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        self.build(&file_contents).map_err(|report| {
            report.with_source_code(miette::NamedSource::new(&self.filename, file_contents))
        })?;
        Ok(())
    }
}

impl BuildCommand {
    fn build(&self, file_contents: &str) -> miette::Result<()> {
        let mut parser = Parser::from(file_contents);
        parser.enable_features(self.preview_features.iter().cloned().collect());

        let ast = parser.parse().into_result().map_err(miette::Report::new)?;
        let mut analyzer = Analyzer::default();
        analyzer.enable_features(self.preview_features.iter().cloned().collect());
        let (mut mir, ty_registry) = analyzer.analyze(&ast).map_err(miette::Report::new)?;
        let mut optimizer = Optimizer::default();
        optimizer.enable_features(self.preview_features.iter().cloned().collect());
        optimizer.optimize(&ty_registry, &mut mir)?;
        println!("{}", mir.display(&ty_registry));
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
        cmd: BuildCommand,
    }

    fn parse(args: &[&str]) -> Result<BuildCommand, clap::Error> {
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
    fn parses_single_preview_feature_short_flag() {
        let cmd = parse(&["-p", "array", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::Array]);
    }

    #[test]
    fn parses_single_preview_feature_long_flag() {
        let cmd = parse(&["--preview", "loop", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::Loop]);
    }

    #[test]
    fn parses_multiple_preview_features_via_repeated_flag() {
        let cmd = parse(&["-p", "array", "-p", "loop", "file.q"]).unwrap();
        assert_eq!(
            cmd.preview_features,
            vec![PreviewFeature::Array, PreviewFeature::Loop]
        );
    }

    #[test]
    fn unknown_preview_feature_is_error() {
        assert!(parse(&["-p", "not-a-real-feature", "file.q"]).is_err());
    }

    #[test]
    fn preview_feature_value_is_kebab_case() {
        let cmd = parse(&["-p", "field-access", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::FieldAccess]);
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse(&["--not-a-flag", "file.q"]).is_err());
    }
}
