use crate::cli::Execute;
use quokka_lang::analyze::Analyzer;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::parse::Parser;

#[derive(clap::Args)]
pub struct BuildCommand {
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
    filename: String,
}

impl Execute for BuildCommand {
    fn execute(self) -> miette::Result<()> {
        let file_contents =
            std::fs::read_to_string(&self.filename).map_err(|e| miette::miette!(e))?;

        self.compile(&file_contents).map_err(|report| {
            report.with_source_code(miette::NamedSource::new(&self.filename, file_contents))
        })?;
        Ok(())
    }
}

impl BuildCommand {
    fn compile(&self, file_contents: &str) -> miette::Result<()> {
        let mut parser = Parser::from(file_contents);
        parser.enable_features(self.preview_features.iter().cloned().collect());

        let ast = parser.parse().into_result().map_err(miette::Report::new)?;
        let (mir, ty_registry) = Analyzer::default()
            .analyze(&ast)
            .map_err(miette::Report::new)?;
        Ok(())
    }
}
