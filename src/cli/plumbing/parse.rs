use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::parse::Parser;

#[derive(clap::Args)]
pub struct ParseCommand {
    filename: String,
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for ParseCommand {
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        let mut parser = Parser::from(file_contents.as_str());
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
