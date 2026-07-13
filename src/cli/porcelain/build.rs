use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;

#[derive(clap::Args)]
pub struct BuildCommand {
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
}

impl Execute for BuildCommand {
    fn execute(self) -> miette::Result<()> {
        println!("{:?}", self.preview_features);
        Ok(())
    }
}
