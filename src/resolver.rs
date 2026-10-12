use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Resolver {
    source_root: PathBuf,
    aliases: Vec<Alias>,
    config_paths: BTreeSet<PathBuf>,
    has_base_url: bool,
}

#[derive(Debug)]
struct Alias {
    pattern: String,
    replacements: Vec<String>,
    config_dir: PathBuf,
}

#[derive(Debug, Default)]
struct CompilerConfig {
    has_base_url: bool,
    paths: BTreeMap<String, PathMapping>,
}

#[derive(Debug)]
struct PathMapping {
    replacements: Vec<String>,
    config_dir: PathBuf,
}

impl Resolver {
    pub fn load(tsconfig: &std::path::Path, source_root: &std::path::Path) -> Result<Self> {
        let source_root = fs::canonicalize(source_root)?;
        if !tsconfig.exists() {
            return Ok(Self {
                source_root,
                aliases: vec![],
                config_paths: BTreeSet::new(),
                has_base_url: false,
            });
        }
        let mut stack = Vec::new();
        let mut config_paths = BTreeSet::new();
        let config = load_config(tsconfig, &mut stack, &mut config_paths)?;
        let has_base_url = config.has_base_url;
        let aliases = config
            .paths
            .into_iter()
            .map(|(pattern, mapping)| Alias {
                pattern,
                replacements: mapping.replacements,
                config_dir: mapping.config_dir,
            })
            .collect();
        Ok(Self {
            source_root,
            aliases,
            config_paths,
            has_base_url,
        })
    }

    pub fn uses_config(&self, path: &std::path::Path) -> bool {
        self.config_paths.contains(path)
            || fs::canonicalize(path)
                .ok()
                .is_some_and(|path| self.config_paths.contains(&path))
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
                let base_url = if self.has_base_url {
                    "; tsconfig compilerOptions.baseUrl is not supported by StyleContract—define this alias with compilerOptions.paths"
                } else {
                    ""
                };
                anyhow!(
                    "stylesheet import '{specifier}' in {} does not match or resolve through a tsconfig paths alias{base_url}",
                    importer.display()
                )
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
                let candidate = alias.config_dir.join(replaced);
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

fn load_config(
    path: &std::path::Path,
    stack: &mut Vec<PathBuf>,
    config_paths: &mut BTreeSet<PathBuf>,
) -> Result<CompilerConfig> {
    let path = fs::canonicalize(path)
        .with_context(|| format!("could not resolve tsconfig {}", path.display()))?;
    if stack.contains(&path) {
        bail!("circular tsconfig extends chain at {}", path.display());
    }
    config_paths.insert(path.clone());
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
        load_config(&parent, stack, config_paths)?
    } else {
        CompilerConfig::default()
    };

    if let Some(options) = value.get("compilerOptions") {
        result.has_base_url |= options.get("baseUrl").is_some();
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
                result.paths.insert(
                    pattern.clone(),
                    PathMapping {
                        replacements,
                        config_dir: directory.to_path_buf(),
                    },
                );
            }
        }
    }
    stack.pop();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn matches_wildcard_aliases() {
        assert_eq!(
            match_pattern("@styles/*", "@styles/button.scss"),
            Some("button.scss")
        );
        assert_eq!(match_pattern("@styles/*", "other/button.scss"), None);
    }

    #[test]
    fn resolves_paths_relative_to_the_config_that_declares_them() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        let configs = temp.path().join("configs");
        fs::create_dir_all(configs.join("styles")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(configs.join("styles/card.module.scss"), ".card {}").unwrap();
        fs::write(
            configs.join("base.json"),
            r#"{ "compilerOptions": { "paths": { "@styles/*": ["styles/*"] } } }"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("tsconfig.json"),
            r#"{ "extends": "./configs/base.json" }"#,
        )
        .unwrap();

        let resolver = Resolver::load(&temp.path().join("tsconfig.json"), temp.path()).unwrap();
        let resolved = resolver
            .resolve(&source.join("component.tsx"), "@styles/card.module.scss")
            .unwrap();
        assert_eq!(
            resolved,
            fs::canonicalize(configs.join("styles/card.module.scss")).unwrap()
        );
    }

    #[test]
    fn ignores_unused_base_url_but_explains_it_when_alias_resolution_fails() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        fs::create_dir_all(source.join("styles")).unwrap();
        fs::write(source.join("styles/card.module.scss"), ".card {}").unwrap();
        fs::write(
            temp.path().join("tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": "./legacy-root",
                    "paths": { "@styles/*": ["src/styles/*"] }
                }
            }"#,
        )
        .unwrap();
        let resolver = Resolver::load(&temp.path().join("tsconfig.json"), &source).unwrap();
        assert!(
            resolver
                .resolve(&source.join("valid.ts"), "@styles/card.module.scss")
                .is_ok()
        );

        let importer = source.join("broken.tsx");
        let error = resolver
            .resolve(&importer, "components/missing.module.scss")
            .unwrap_err()
            .to_string();
        assert!(error.contains(&importer.display().to_string()), "{error}");
        assert!(error.contains("baseUrl is not supported"), "{error}");
        assert!(error.contains("compilerOptions.paths"), "{error}");
    }

    #[test]
    fn resolves_paths_without_base_url() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("theme.module.css"), ".theme {}").unwrap();
        fs::write(
            temp.path().join("tsconfig.json"),
            r#"{ "compilerOptions": { "paths": { "@/*": ["src/*"] } } }"#,
        )
        .unwrap();
        let resolver = Resolver::load(&temp.path().join("tsconfig.json"), &source).unwrap();
        assert_eq!(
            resolver
                .resolve(Path::new("component.ts"), "@/theme.module.css")
                .unwrap(),
            fs::canonicalize(source.join("theme.module.css")).unwrap()
        );
    }
}
