use std::path::PathBuf;

use clap::Parser;

use crate::naming::Convention;

#[derive(Debug, Parser)]
#[command(name = "style-contract", version, about)]
pub struct Cli {
    /// Naming convention shared by TypeScript and stylesheets.
    #[arg(long, value_enum)]
    pub convention: Convention,

    /// Source tree to analyze.
    #[arg(long, default_value = "./src")]
    pub source: PathBuf,

    /// Folder below the source tree to exclude (repeatable).
    #[arg(long, value_name = "PATH")]
    pub exclude: Vec<PathBuf>,

    /// Excluded folder to explicitly include again (repeatable).
    #[arg(long, value_name = "PATH")]
    pub include: Vec<PathBuf>,

    /// Ignore :export declarations and references to them.
    #[arg(long)]
    pub ignore_exports: bool,

    /// Override a rule as RULE:error, RULE:warning, or RULE:off.
    #[arg(long, value_name = "RULE:SEVERITY")]
    pub rule: Vec<String>,

    /// TypeScript config used for baseUrl and paths aliases.
    #[arg(long, default_value = "./tsconfig.json")]
    pub tsconfig: PathBuf,
}
