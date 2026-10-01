use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use serde::Deserialize;

use crate::naming::Convention;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputStyle {
    Minimal,
    Rich,
}

#[derive(Debug, Parser)]
#[command(name = "style-contract", version, about)]
pub struct Cli {
    /// Naming convention shared by TypeScript and stylesheets.
    #[arg(long, value_enum)]
    pub convention: Option<Convention>,

    /// Explicit JSON5 configuration file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Diagnostic output style.
    #[arg(long, value_enum)]
    pub output: Option<OutputStyle>,

    /// Source tree to analyze.
    #[arg(long)]
    pub source: Option<PathBuf>,

    /// Folder below the source tree to exclude (repeatable).
    #[arg(long, value_name = "PATH")]
    pub exclude: Vec<PathBuf>,

    /// Excluded folder to explicitly include again (repeatable).
    #[arg(long, value_name = "PATH")]
    pub include: Vec<PathBuf>,

    /// Ignore :export declarations and references to them.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true
    )]
    pub ignore_exports: Option<bool>,

    /// Override a rule as RULE:error, RULE:warning, or RULE:off.
    #[arg(long, value_name = "RULE:SEVERITY")]
    pub rule: Vec<String>,

    /// TypeScript config used for baseUrl and paths aliases.
    #[arg(long)]
    pub tsconfig: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_exports_accepts_switch_and_explicit_false() {
        let enabled = Cli::try_parse_from(["style-contract", "--ignore-exports"]).unwrap();
        assert_eq!(enabled.ignore_exports, Some(true));
        let disabled = Cli::try_parse_from(["style-contract", "--ignore-exports=false"]).unwrap();
        assert_eq!(disabled.ignore_exports, Some(false));
    }
}
