pub mod analysis;
pub mod cli;
pub mod config;
pub mod diagnostic;
pub mod discovery;
pub mod naming;
pub mod resolver;
pub mod stylesheet;
pub mod typescript;
pub mod workspace;

use std::path::PathBuf;

use anyhow::Result;
use config::Config;
use diagnostic::{Diagnostic, UnusedSymbol};

#[derive(Debug, Clone)]
pub struct RunResult {
    pub diagnostics: Vec<Diagnostic>,
    pub unused_symbols: Vec<UnusedSymbol>,
    pub has_errors: bool,
}

pub fn run(config: &Config) -> Result<RunResult> {
    let files = discovery::discover(config)?;
    let styles = stylesheet::parse_all(&files.stylesheets, config.ignore_exports)?;
    let resolver = resolver::Resolver::load(&config.tsconfig, &config.source)?;
    let modules = typescript::parse_all(&files.typescript, &resolver)?;
    let analysis = analysis::analyze(config, styles, modules);
    let mut diagnostics = analysis.diagnostics;
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    let has_errors = diagnostics.iter().any(|d| d.severity.is_error());
    Ok(RunResult {
        diagnostics,
        unused_symbols: analysis.unused_symbols,
        has_errors,
    })
}

pub use workspace::WorkspaceIndex;

pub fn display_path(path: &std::path::Path, cwd: &std::path::Path) -> PathBuf {
    path.strip_prefix(cwd).unwrap_or(path).to_path_buf()
}
