use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::parse::Parser;
use tracing::instrument;

#[derive(clap::Args)]
pub struct ParseCommand {
    filename: String,
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for ParseCommand {
    #[instrument(skip(self))]
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        let mut parser = Parser::from(file_contents.as_str());
        parser.enable_features(self.preview_features.iter().cloned().collect());
        let ast = parser.parse().into_result().map_err(|e| {
            miette::Report::new(e)
                .with_source_code(miette::NamedSource::new(&self.filename, file_contents))
        })?;
        println!(
            "{}",
            ron::ser::to_string_pretty(
                &ast,
                ron::ser::PrettyConfig::new()
                    .struct_names(true)
                    .indentor("  ")
            )
            .map_err(miette::Report::from_err)?,
        );

        Ok(())
    }
}
