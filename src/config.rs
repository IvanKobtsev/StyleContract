use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    cli::{Cli, OutputStyle},
    diagnostic::{LEGACY_NAMING_RULE, LEGACY_RULE_ALIASES, RULES, Severity},
    naming::Convention,
};

#[derive(Debug)]
pub struct Config {
    pub cwd: PathBuf,
    pub convention: Convention,
    pub output: OutputStyle,
    pub source: PathBuf,
    pub exclude: Vec<PathBuf>,
    pub include: Vec<PathBuf>,
    pub ignore_exports: bool,
    pub tsconfig: PathBuf,
    severities: BTreeMap<&'static str, Severity>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileConfig {
    convention: Option<Convention>,
    output: Option<OutputStyle>,
    source: Option<PathBuf>,
    #[serde(default)]
    exclude: Vec<PathBuf>,
    #[serde(default)]
    include: Vec<PathBuf>,
    ignore_exports: Option<bool>,
    tsconfig: Option<PathBuf>,
    #[serde(default)]
    rules: BTreeMap<String, String>,
}

impl Config {
    pub fn from_cli(cli: Cli, cwd: PathBuf) -> Result<Self> {
        let cwd = fs::canonicalize(&cwd).context("could not resolve the working directory")?;
        let (file, config_base) = load_file(cli.config.as_deref(), &cwd)?;

        let convention = cli.convention.or(file.convention).ok_or_else(|| {
            anyhow::anyhow!("--convention is required when it is not set in --config")
        })?;
        let output = cli.output.or(file.output).unwrap_or(OutputStyle::Rich);
        let ignore_exports = cli.ignore_exports.or(file.ignore_exports).unwrap_or(false);

        let (source_value, source_base) = match cli.source {
            Some(path) => (path, cwd.as_path()),
            None => match file.source {
                Some(path) => (path, config_base.as_path()),
                None => (PathBuf::from("./src"), cwd.as_path()),
            },
        };
        let source = resolve_existing(source_base, &source_value, "source")?;
        if !source.is_dir() {
            bail!("source path is not a directory: {}", source.display());
        }

        let (exclude_values, exclude_base) = if cli.exclude.is_empty() {
            (file.exclude, config_base.as_path())
        } else {
            (cli.exclude, cwd.as_path())
        };
        let (include_values, include_base) = if cli.include.is_empty() {
            (file.include, config_base.as_path())
        } else {
            (cli.include, cwd.as_path())
        };
        let exclude = resolve_folders(exclude_base, &source, exclude_values, "exclude")?;
        let include = resolve_folders(include_base, &source, include_values, "include")?;

        let (tsconfig_value, tsconfig_base) = match cli.tsconfig {
            Some(path) => (path, cwd.as_path()),
            None => match file.tsconfig {
                Some(path) => (path, config_base.as_path()),
                None => (PathBuf::from("./tsconfig.json"), cwd.as_path()),
            },
        };
        let tsconfig = absolute(tsconfig_base, &tsconfig_value);
        if tsconfig.exists() && !tsconfig.is_file() {
            bail!("tsconfig path is not a file: {}", tsconfig.display());
        }

        let overrides = if cli.rule.is_empty() {
            file.rules.into_iter().collect()
        } else {
            parse_cli_rules(cli.rule)?
        };
        let severities = build_severities(overrides)?;

        Ok(Self {
            cwd,
            convention,
            output,
            source,
            exclude,
            include,
            ignore_exports,
            tsconfig,
            severities,
        })
    }

    pub fn severity(&self, rule: &'static str) -> Severity {
        self.severities[rule]
    }
}

fn load_file(
    path: Option<&std::path::Path>,
    cwd: &std::path::Path,
) -> Result<(FileConfig, PathBuf)> {
    let Some(path) = path else {
        return Ok((FileConfig::default(), cwd.to_path_buf()));
    };
    let unresolved = absolute(cwd, path);
    let path = fs::canonicalize(&unresolved)
        .with_context(|| format!("config path does not exist: {}", unresolved.display()))?;
    if !path.is_file() {
        bail!("config path is not a file: {}", path.display());
    }
    let source = fs::read_to_string(&path)
        .with_context(|| format!("could not read config {}", path.display()))?;
    let config: FileConfig = json5::from_str(&source)
        .with_context(|| format!("could not parse config {}", path.display()))?;
    let base = path
        .parent()
        .expect("canonical file has a parent")
        .to_path_buf();
    Ok((config, base))
}

fn parse_cli_rules(values: Vec<String>) -> Result<Vec<(String, String)>> {
    let mut seen = BTreeMap::<String, String>::new();
    let mut parsed = Vec::new();
    for value in values {
        let (rule, severity) = value.split_once(':').ok_or_else(|| {
            anyhow::anyhow!("invalid rule override '{value}'; expected RULE:SEVERITY")
        })?;
        if let Some(previous) = seen.insert(rule.to_owned(), severity.to_owned())
            && previous != severity
        {
            bail!("conflicting overrides supplied for rule '{rule}'");
        }
        parsed.push((rule.to_owned(), severity.to_owned()));
    }
    Ok(parsed)
}

fn build_severities(overrides: Vec<(String, String)>) -> Result<BTreeMap<&'static str, Severity>> {
    let mut severities = BTreeMap::from([
        ("missing-symbol", Severity::Error),
        ("unused-class", Severity::Error),
        ("unused-export", Severity::Error),
        ("naming-convention-local", Severity::Error),
        ("naming-convention-global", Severity::Warning),
        ("dynamic-reference", Severity::Warning),
    ]);
    let mut parsed = Vec::new();
    for (rule, value) in overrides {
        if rule != LEGACY_NAMING_RULE
            && !RULES.contains(&rule.as_str())
            && !LEGACY_RULE_ALIASES.iter().any(|(alias, _)| *alias == rule)
        {
            bail!("unknown rule '{rule}'");
        }
        let severity = Severity::parse(&value).ok_or_else(|| {
            anyhow::anyhow!("invalid severity for '{rule}'; expected error, warning, or off")
        })?;
        parsed.push((rule, severity));
    }
    if let Some((_, severity)) = parsed.iter().find(|(rule, _)| rule == LEGACY_NAMING_RULE) {
        severities.insert("naming-convention-local", *severity);
        severities.insert("naming-convention-global", *severity);
    }
    for (alias, target) in LEGACY_RULE_ALIASES {
        if let Some((_, severity)) = parsed.iter().find(|(rule, _)| rule == alias) {
            severities.insert(target, *severity);
        }
    }
    for (rule, severity) in parsed {
        if rule == LEGACY_NAMING_RULE || LEGACY_RULE_ALIASES.iter().any(|(alias, _)| *alias == rule)
        {
            continue;
        }
        let known = RULES
            .iter()
            .copied()
            .find(|candidate| *candidate == rule)
            .expect("rule was validated");
        severities.insert(known, severity);
    }
    Ok(severities)
}

fn absolute(base: &std::path::Path, value: &std::path::Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    }
}

fn resolve_existing(
    base: &std::path::Path,
    value: &std::path::Path,
    label: &str,
) -> Result<PathBuf> {
    let path = absolute(base, value);
    fs::canonicalize(&path)
        .with_context(|| format!("{label} path does not exist: {}", path.display()))
}

fn resolve_folders(
    base: &std::path::Path,
    source: &std::path::Path,
    values: Vec<PathBuf>,
    label: &str,
) -> Result<Vec<PathBuf>> {
    values
        .into_iter()
        .map(|value| {
            let path = resolve_existing(base, &value, label)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> Cli {
        Cli {
            convention: None,
            config: None,
            output: None,
            source: None,
            exclude: vec![],
            include: vec![],
            ignore_exports: None,
            rule: vec![],
            tsconfig: None,
        }
    }

    #[test]
    fn loads_json5_and_resolves_paths_from_config_directory() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let config_dir = project.join("config");
        let source = config_dir.join("web-src");
        let excluded = source.join("generated");
        fs::create_dir_all(&excluded).unwrap();
        fs::write(
            config_dir.join("style-contract.json5"),
            r#"{
                // JSON5 comments and trailing commas are supported.
                convention: "camel-to-kebab",
                output: "minimal",
                source: "./web-src",
                exclude: ["./web-src/generated",],
                ignoreExports: true,
                tsconfig: "./tsconfig.web.json",
                rules: { "naming-convention-global": "off", },
            }"#,
        )
        .unwrap();
        fs::write(config_dir.join("tsconfig.web.json"), "{}").unwrap();
        let mut cli = empty_cli();
        cli.config = Some(PathBuf::from("config/style-contract.json5"));
        let config = Config::from_cli(cli, project).unwrap();
        assert_eq!(config.convention, Convention::CamelToKebab);
        assert_eq!(config.output, OutputStyle::Minimal);
        assert!(config.ignore_exports);
        assert_eq!(config.source, fs::canonicalize(source).unwrap());
        assert_eq!(config.exclude, vec![fs::canonicalize(excluded).unwrap()]);
        assert_eq!(config.severity("naming-convention-global"), Severity::Off);
    }

    #[test]
    fn cli_values_replace_config_values() {
        let temp = tempfile::tempdir().unwrap();
        let config_src = temp.path().join("configured-src");
        let cli_src = temp.path().join("cli-src");
        let cli_exclude = cli_src.join("excluded");
        fs::create_dir_all(&config_src).unwrap();
        fs::create_dir_all(&cli_exclude).unwrap();
        fs::write(
            temp.path().join("style-contract.json5"),
            r#"{
                convention: "kebab-case",
                output: "minimal",
                source: "./configured-src",
                ignoreExports: true,
                rules: { "unused-class": "off" },
            }"#,
        )
        .unwrap();
        let mut cli = empty_cli();
        cli.config = Some("style-contract.json5".into());
        cli.convention = Some(Convention::CamelCase);
        cli.output = Some(OutputStyle::Rich);
        cli.source = Some("cli-src".into());
        cli.exclude = vec!["cli-src/excluded".into()];
        cli.ignore_exports = Some(false);
        cli.tsconfig = Some("cli-tsconfig.json".into());
        cli.rule = vec!["unused-export:warning".into()];
        let config = Config::from_cli(cli, temp.path().to_path_buf()).unwrap();
        assert_eq!(config.convention, Convention::CamelCase);
        assert_eq!(config.output, OutputStyle::Rich);
        assert!(!config.ignore_exports);
        assert_eq!(config.source, fs::canonicalize(cli_src).unwrap());
        assert_eq!(config.exclude, vec![fs::canonicalize(cli_exclude).unwrap()]);
        assert_eq!(config.severity("unused-class"), Severity::Error);
        assert_eq!(config.severity("unused-export"), Severity::Warning);
    }

    #[test]
    fn rejects_unknown_fields_and_missing_convention() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("bad.json5"),
            r#"{ convention: "camel-case", mystery: true }"#,
        )
        .unwrap();
        let mut cli = empty_cli();
        cli.config = Some("bad.json5".into());
        assert!(
            Config::from_cli(cli, temp.path().to_path_buf())
                .unwrap_err()
                .to_string()
                .contains("could not parse config")
        );
        assert!(
            Config::from_cli(empty_cli(), temp.path().to_path_buf())
                .unwrap_err()
                .to_string()
                .contains("--convention is required")
        );

        fs::write(temp.path().join("malformed.json5"), "{ convention:").unwrap();
        let mut malformed = empty_cli();
        malformed.config = Some("malformed.json5".into());
        assert!(
            Config::from_cli(malformed, temp.path().to_path_buf())
                .unwrap_err()
                .to_string()
                .contains("could not parse config")
        );
    }

    #[test]
    fn legacy_rule_aliases_apply_before_canonical_rules() {
        for overrides in [
            vec![
                ("naming-convention".to_owned(), "off".to_owned()),
                ("naming-convention-global".to_owned(), "warning".to_owned()),
            ],
            vec![
                ("naming-convention-global".to_owned(), "warning".to_owned()),
                ("naming-convention".to_owned(), "off".to_owned()),
            ],
        ] {
            let severities = build_severities(overrides).unwrap();
            assert_eq!(severities["naming-convention-local"], Severity::Off);
            assert_eq!(severities["naming-convention-global"], Severity::Warning);
        }

        for overrides in [
            vec![
                ("no-unused-classes".to_owned(), "off".to_owned()),
                ("unused-class".to_owned(), "warning".to_owned()),
            ],
            vec![
                ("unused-class".to_owned(), "warning".to_owned()),
                ("no-unused-classes".to_owned(), "off".to_owned()),
            ],
        ] {
            let severities = build_severities(overrides).unwrap();
            assert_eq!(severities["unused-class"], Severity::Warning);
        }
    }
}
