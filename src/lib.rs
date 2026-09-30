pub mod analysis;
pub mod cli;
pub mod config;
pub mod diagnostic;
pub mod discovery;
pub mod naming;
pub mod resolver;
pub mod stylesheet;
pub mod typescript;

use std::path::PathBuf;

use anyhow::Result;
use config::Config;
use diagnostic::Diagnostic;

#[derive(Debug)]
pub struct RunResult {
    pub diagnostics: Vec<Diagnostic>,
    pub has_errors: bool,
}

pub fn run(config: &Config) -> Result<RunResult> {
    let files = discovery::discover(config)?;
    let styles = stylesheet::parse_all(&files.stylesheets, config.ignore_exports)?;
    let resolver = resolver::Resolver::load(&config.tsconfig, &config.source)?;
    let modules = typescript::parse_all(&files.typescript, &resolver)?;
    let mut diagnostics = analysis::analyze(config, styles, modules);
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    let has_errors = diagnostics.iter().any(|d| d.severity.is_error());
    Ok(RunResult {
        diagnostics,
        has_errors,
    })
}

pub fn display_path(path: &std::path::Path, cwd: &std::path::Path) -> PathBuf {
    path.strip_prefix(cwd).unwrap_or(path).to_path_buf()
}
