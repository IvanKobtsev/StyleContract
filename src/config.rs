use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    cli::Cli,
    diagnostic::{RULES, Severity},
    naming::Convention,
};

#[derive(Debug)]
pub struct Config {
    pub cwd: PathBuf,
    pub convention: Convention,
    pub source: PathBuf,
    pub exclude: Vec<PathBuf>,
    pub include: Vec<PathBuf>,
    pub ignore_exports: bool,
    pub tsconfig: PathBuf,
    severities: BTreeMap<&'static str, Severity>,
}

impl Config {
    pub fn from_cli(cli: Cli, cwd: PathBuf) -> Result<Self> {
        let cwd = fs::canonicalize(&cwd).context("could not resolve the working directory")?;
        let source = resolve_existing(&cwd, &cli.source, "source")?;
        if !source.is_dir() {
            bail!("source path is not a directory: {}", source.display());
        }
        let exclude = resolve_folders(&cwd, &source, cli.exclude, "exclude")?;
        let include = resolve_folders(&cwd, &source, cli.include, "include")?;
        let tsconfig = absolute(&cwd, &cli.tsconfig);
        if tsconfig.exists() && !tsconfig.is_file() {
            bail!("tsconfig path is not a file: {}", tsconfig.display());
        }

        let mut severities = BTreeMap::from([
            (RULES[0], Severity::Error),
            (RULES[1], Severity::Error),
            (RULES[2], Severity::Error),
            (RULES[3], Severity::Error),
            (RULES[4], Severity::Warning),
        ]);
        let mut specified = BTreeMap::<String, Severity>::new();
        for override_value in cli.rule {
            let (rule, severity) = override_value.split_once(':').ok_or_else(|| {
                anyhow::anyhow!("invalid rule override '{override_value}'; expected RULE:SEVERITY")
            })?;
            let known = RULES
                .iter()
                .copied()
                .find(|candidate| *candidate == rule)
                .ok_or_else(|| anyhow::anyhow!("unknown rule '{rule}'"))?;
            let severity = Severity::parse(severity).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid severity in '{override_value}'; expected error, warning, or off"
                )
            })?;
            if let Some(previous) = specified.insert(rule.to_owned(), severity)
                && previous != severity
            {
                bail!("conflicting overrides supplied for rule '{rule}'");
            }
            severities.insert(known, severity);
        }

        Ok(Self {
            cwd,
            convention: cli.convention,
            source,
            exclude,
            include,
            ignore_exports: cli.ignore_exports,
            tsconfig,
            severities,
        })
    }

    pub fn severity(&self, rule: &'static str) -> Severity {
        self.severities[rule]
    }
}

fn absolute(cwd: &std::path::Path, value: &std::path::Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        cwd.join(value)
    }
}

fn resolve_existing(
    cwd: &std::path::Path,
    value: &std::path::Path,
    label: &str,
) -> Result<PathBuf> {
    let path = absolute(cwd, value);
    fs::canonicalize(&path)
        .with_context(|| format!("{label} path does not exist: {}", path.display()))
}

fn resolve_folders(
    cwd: &std::path::Path,
    source: &std::path::Path,
    values: Vec<PathBuf>,
    label: &str,
) -> Result<Vec<PathBuf>> {
    values
        .into_iter()
        .map(|value| {
            let path = resolve_existing(cwd, &value, label)?;
            if !path.is_dir() {
                bail!("{label} path is not a directory: {}", path.display());
            }
            if !path.starts_with(source) {
                bail!("{label} path must be inside --source: {}", path.display());
            }
            Ok(path)
        })
        .collect()
}
