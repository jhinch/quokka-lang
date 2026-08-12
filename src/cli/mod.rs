pub mod plumbing;
pub mod porcelain;

pub trait Execute {
    fn execute(self) -> miette::Result<()>;
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,

    /// Increase logging verbosity (-v for debug, -vv for trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Cli {
    pub fn tracing_level(&self) -> tracing::Level {
        match self.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    }
}

impl Execute for Cli {
    fn execute(self) -> miette::Result<()> {
        self.command.execute()
    }
}

#[derive(clap::Subcommand)]
pub enum CliCommand {
    #[command(flatten)]
    Plumbing(plumbing::PlumbingCommand),
    #[command(flatten)]
    Porcelain(porcelain::PorcelainCommand),
}

impl Execute for CliCommand {
    fn execute(self) -> miette::Result<()> {
        match self {
            CliCommand::Plumbing(command) => command.execute(),
            CliCommand::Porcelain(command) => command.execute(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = vec!["cmd"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full)
    }

    #[test]
    fn missing_subcommand_is_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn unknown_subcommand_is_error() {
        assert!(parse(&["unknown", "file.q"]).is_err());
    }

    #[test]
    fn routes_build_to_porcelain_build() {
        let cli = parse(&["build", "file.q"]).unwrap();
        assert!(matches!(
            cli.command,
            CliCommand::Porcelain(porcelain::PorcelainCommand::Build(_))
        ));
    }

    #[test]
    fn routes_lex_to_plumbing_lex() {
        let cli = parse(&["lex", "file.q"]).unwrap();
        assert!(matches!(
            cli.command,
            CliCommand::Plumbing(plumbing::PlumbingCommand::Lex(_))
        ));
    }

    #[test]
    fn routes_parse_to_plumbing_parse() {
        let cli = parse(&["parse", "file.q"]).unwrap();
        assert!(matches!(
            cli.command,
            CliCommand::Plumbing(plumbing::PlumbingCommand::Parse(_))
        ));
    }

    #[test]
    fn default_verbosity_is_warn() {
        let cli = parse(&["build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::WARN);
    }

    #[test]
    fn single_v_short_flag_is_info() {
        let cli = parse(&["-v", "build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::INFO);
    }

    #[test]
    fn double_v_short_flag_is_debug() {
        let cli = parse(&["-vv", "build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::DEBUG);
    }

    #[test]
    fn triple_v_short_flag_is_trace() {
        let cli = parse(&["-vvv", "build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::TRACE);
    }

    #[test]
    fn beyond_triple_v_stays_trace() {
        let cli = parse(&["-vvvv", "build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::TRACE);
    }

    #[test]
    fn repeated_long_verbose_flag_accumulates() {
        let cli = parse(&["--verbose", "--verbose", "build", "file.q"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::DEBUG);
    }

    #[test]
    fn verbose_flag_is_global_and_works_after_subcommand() {
        let cli = parse(&["build", "file.q", "-v"]).unwrap();
        assert_eq!(cli.tracing_level(), tracing::Level::INFO);
    }
}
