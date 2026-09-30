use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Resolver {
    source_root: PathBuf,
    aliases: Vec<Alias>,
}

#[derive(Debug)]
struct Alias {
    pattern: String,
    replacements: Vec<String>,
    base_url: PathBuf,
}

#[derive(Debug, Default)]
struct CompilerConfig {
    base_url: Option<PathBuf>,
    paths: BTreeMap<String, Vec<String>>,
}

impl Resolver {
    pub fn load(tsconfig: &std::path::Path, source_root: &std::path::Path) -> Result<Self> {
        let source_root = fs::canonicalize(source_root)?;
        if !tsconfig.exists() {
            return Ok(Self {
                source_root,
                aliases: vec![],
            });
        }
        let mut stack = Vec::new();
        let config = load_config(tsconfig, &mut stack)?;
        let default_base = tsconfig
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let base_url = config
            .base_url
            .unwrap_or_else(|| default_base.to_path_buf());
        let aliases = config
            .paths
            .into_iter()
            .map(|(pattern, replacements)| Alias {
                pattern,
                replacements,
                base_url: base_url.clone(),
            })
            .collect();
        Ok(Self {
            source_root,
            aliases,
        })
    }

    pub fn resolve(&self, importer: &std::path::Path, specifier: &str) -> Result<PathBuf> {
        if !matches!(
            std::path::Path::new(specifier)
                .extension()
                .and_then(|value| value.to_str()),
            Some("css" | "scss")
        ) {
            bail!("stylesheet import must have a .css or .scss extension: {specifier}");
        }
        let candidate = if specifier.starts_with('.') {
            importer
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(specifier)
        } else {
            self.resolve_alias(specifier).ok_or_else(|| {
                anyhow!("stylesheet import '{specifier}' does not match a tsconfig paths alias")
            })?
        };
        let resolved = fs::canonicalize(&candidate).with_context(|| {
            format!(
                "could not resolve stylesheet import '{specifier}' from {}",
                importer.display()
            )
        })?;
        if !resolved.starts_with(&self.source_root) {
            bail!(
                "stylesheet import resolves outside --source: {}",
                resolved.display()
            );
        }
        Ok(resolved)
    }

    fn resolve_alias(&self, specifier: &str) -> Option<PathBuf> {
        for alias in &self.aliases {
            let Some(captured) = match_pattern(&alias.pattern, specifier) else {
                continue;
            };
            for replacement in &alias.replacements {
                let replaced = if replacement.contains('*') {
                    replacement.replacen('*', captured, 1)
                } else {
                    replacement.clone()
                };
                let candidate = alias.base_url.join(replaced);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

fn match_pattern<'a>(pattern: &str, value: &'a str) -> Option<&'a str> {
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        value
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
    } else if pattern == value {
        Some("")
    } else {
        None
    }
}

fn load_config(path: &std::path::Path, stack: &mut Vec<PathBuf>) -> Result<CompilerConfig> {
    let path = fs::canonicalize(path)
        .with_context(|| format!("could not resolve tsconfig {}", path.display()))?;
    if stack.contains(&path) {
        bail!("circular tsconfig extends chain at {}", path.display());
    }
    stack.push(path.clone());
    let source = fs::read_to_string(&path)
        .with_context(|| format!("could not read tsconfig {}", path.display()))?;
    let value: Value = json5::from_str(&source)
        .with_context(|| format!("could not parse tsconfig {}", path.display()))?;
    let directory = path.parent().expect("canonical file has a parent");
    let mut result = if let Some(extends) = value.get("extends").and_then(Value::as_str) {
        if !extends.starts_with('.') && !std::path::Path::new(extends).is_absolute() {
            bail!("only local tsconfig extends paths are supported: {extends}");
        }
        let mut parent = directory.join(extends);
        if parent.extension().is_none() {
            parent.set_extension("json");
        }
        load_config(&parent, stack)?
    } else {
        CompilerConfig::default()
    };

    if let Some(options) = value.get("compilerOptions") {
        if let Some(base_url) = options.get("baseUrl").and_then(Value::as_str) {
            result.base_url = Some(directory.join(base_url));
        }
        if let Some(paths) = options.get("paths").and_then(Value::as_object) {
            for (pattern, replacements) in paths {
                let replacements = replacements
                    .as_array()
                    .ok_or_else(|| anyhow!("tsconfig path '{pattern}' must contain an array"))?
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            anyhow!("tsconfig path '{pattern}' contains a non-string replacement")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                result.paths.insert(pattern.clone(), replacements);
            }
        }
    }
    stack.pop();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_wildcard_aliases() {
        assert_eq!(
            match_pattern("@styles/*", "@styles/button.scss"),
            Some("button.scss")
        );
        assert_eq!(match_pattern("@styles/*", "other/button.scss"), None);
    }
}
