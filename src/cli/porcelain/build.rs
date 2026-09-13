use crate::cli::Execute;
use quokka_lang::feature::PreviewFeature;
use quokka_lang::pipeline::PipelineConfig;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tracing::instrument;

#[derive(clap::Args)]
pub struct BuildCommand {
    /// Enable one or more preview features
    #[arg(short = 'p', long = "preview")]
    preview_features: Vec<PreviewFeature>,
    filename: String,
}

impl Execute for BuildCommand {
    #[instrument(skip_all)]
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
        let entrypoint_path = PathBuf::from(&self.filename);
        let build_dir = entrypoint_path
            .parent()
            .ok_or_else(|| miette::miette!("Cannot determine parent directory"))?
            .join("build");

        if !build_dir.is_dir() {
            std::fs::create_dir(&build_dir).map_err(|e| miette::miette!(e))?;
        }

        let entrypoint_stem = entrypoint_path
            .file_stem()
            .ok_or_else(|| miette::miette!("Cannot determine file stem"))?;
        let obj_path = build_dir.join(format!("{}.obj", entrypoint_stem.display()));

        let data = PipelineConfig::default()
            .with_features(self.preview_features.iter().cloned().collect())
            .start()
            .parse(file_contents)?
            .analyze()?
            .optimize()?
            .codegen()?
            .allocate()
            .encode();

        let mut obj_file = BufWriter::new(File::create(&obj_path).map_err(|e| miette::miette!(e))?);
        quokka_lang_obj::write(&mut obj_file, &data).map_err(|e| miette::miette!(e))?;
        obj_file.flush().map_err(|e| miette::miette!(e))?;

        let gb_path = build_dir.join(format!("{}.gb", entrypoint_stem.display()));
        let sym_path = build_dir.join(format!("{}.sym", entrypoint_stem.display()));
        std::process::Command::new("rgblink")
            .arg("--output")
            .arg(&gb_path)
            .arg("--sym")
            .arg(&sym_path)
            .arg(&obj_path)
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|e| miette::miette!(e))?;

        std::process::Command::new("rgbfix")
            .arg("-v")
            .arg("-p")
            .arg("0xFF")
            .arg(&gb_path)
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|e| miette::miette!(e))?;

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
        let cmd = parse(&["-p", "example-preview", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::ExamplePreview]);
    }

    #[test]
    fn parses_single_preview_feature_long_flag() {
        let cmd = parse(&["--preview", "example-preview", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::ExamplePreview]);
    }

    #[test]
    fn parses_multiple_preview_features_via_repeated_flag() {
        let cmd = parse(&["-p", "example-preview", "-p", "no-startup-code", "file.q"]).unwrap();
        assert_eq!(
            cmd.preview_features,
            vec![
                PreviewFeature::ExamplePreview,
                PreviewFeature::NoStartupCode
            ]
        );
    }

    #[test]
    fn unknown_preview_feature_is_error() {
        assert!(parse(&["-p", "not-a-real-feature", "file.q"]).is_err());
    }

    #[test]
    fn preview_feature_value_is_kebab_case() {
        let cmd = parse(&["-p", "example-preview", "file.q"]).unwrap();
        assert_eq!(cmd.preview_features, vec![PreviewFeature::ExamplePreview]);
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse(&["--not-a-flag", "file.q"]).is_err());
    }
}
