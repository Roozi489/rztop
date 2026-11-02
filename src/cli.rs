// Modified for rztop on 2026-11-02: fork identity and configuration isolation.
use clap::Parser;
use std::path::PathBuf;

/// A terminal-based system monitor for Windows, inspired by btop.
#[derive(Parser, Debug)]
#[command(name = "rztop", version, about)]
pub struct Cli {
    /// Path to config file.
    #[arg(short = 'c', long = "config")]
    pub config_file: Option<PathBuf>,

    /// Import an existing rtop config into rztop's default location without changing the original.
    #[arg(long = "import-config", conflicts_with = "config_file")]
    pub import_config: Option<PathBuf>,

    /// Set initial process filter.
    #[arg(short = 'f', long = "filter")]
    pub filter: Option<String>,

    /// Update rate in milliseconds (minimum 100).
    #[arg(short = 'u', long = "update", value_parser = clap::value_parser!(u32).range(100..))]
    pub update_ms: Option<u32>,

    /// Print default config to stdout and exit.
    #[arg(long = "default-config")]
    pub default_config: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_args() {
        let cli = Cli::parse_from(["rztop"]);
        assert!(cli.config_file.is_none());
        assert!(cli.filter.is_none());
        assert!(cli.update_ms.is_none());
    }

    #[test]
    fn parse_config_file() {
        let cli = Cli::parse_from(["rztop", "-c", "my.conf"]);
        assert_eq!(cli.config_file.unwrap(), PathBuf::from("my.conf"));
    }

    #[test]
    fn parse_filter() {
        let cli = Cli::parse_from(["rztop", "-f", "chrome"]);
        assert_eq!(cli.filter.unwrap(), "chrome");
    }

    #[test]
    fn parse_update_ms_minimum() {
        let result = Cli::try_parse_from(["rztop", "-u", "50"]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_default_config() {
        let cli = Cli::parse_from(["rztop", "--default-config"]);
        assert!(cli.default_config);
    }

    #[test]
    fn parse_update_valid() {
        let cli = Cli::parse_from(["rztop", "-u", "500"]);
        assert_eq!(cli.update_ms.unwrap(), 500);
    }
}
