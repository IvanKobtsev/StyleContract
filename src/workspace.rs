use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    RunResult, analysis,
    config::Config,
    discovery,
    resolver::Resolver,
    stylesheet::{self, Stylesheet},
    typescript::{self, TypeScriptModule},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    Unchanged,
    Updated,
    FullReloadRequired,
}

/// A fully parsed StyleContract workspace with editor-owned text overlays.
pub struct WorkspaceIndex {
    workspace_root: PathBuf,
    config_path: PathBuf,
    config: Config,
    resolver: Resolver,
    stylesheets: BTreeMap<PathBuf, Stylesheet>,
    modules: BTreeMap<PathBuf, TypeScriptModule>,
    overlays: BTreeMap<PathBuf, String>,
    cached_result: Option<RunResult>,
    #[cfg(test)]
    analysis_runs: usize,
}

impl WorkspaceIndex {
    pub fn load(workspace_root: PathBuf, config_path: PathBuf) -> Result<Self> {
        let workspace_root =
            fs::canonicalize(&workspace_root).context("could not resolve the workspace root")?;
        let config = Config::from_path(workspace_root.clone(), config_path.clone())?;
        let config_path = fs::canonicalize(&config_path)
            .with_context(|| format!("could not resolve config {}", config_path.display()))?;
        let resolver = Resolver::load(&config.tsconfig, &config.source)?;
        let files = discovery::discover(&config)?;
        let stylesheets = stylesheet::parse_all(&files.stylesheets, config.ignore_exports)?
            .into_iter()
            .map(|stylesheet| (stylesheet.path.clone(), stylesheet))
            .collect();
        let modules = files
            .typescript
            .iter()
            .cloned()
            .zip(typescript::parse_all(&files.typescript, &resolver)?)
            .collect();
        Ok(Self {
            workspace_root,
            config_path,
            config,
            resolver,
            stylesheets,
            modules,
            overlays: BTreeMap::new(),
            cached_result: None,
            #[cfg(test)]
            analysis_runs: 0,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn update_document(&mut self, path: PathBuf, text: String) -> Result<RefreshOutcome> {
        let path = normalize_existing_or_absolute(&path)?;
        if !self.included(&path) || !supported(&path) {
            return Ok(RefreshOutcome::Unchanged);
        }
        if self.overlays.get(&path) == Some(&text) {
            return Ok(RefreshOutcome::Unchanged);
        }
        self.parse_and_store(&path, Some(&text))?;
        self.overlays.insert(path, text);
        self.invalidate();
        Ok(RefreshOutcome::Updated)
    }

    pub fn close_document(&mut self, path: &Path) -> Result<RefreshOutcome> {
        let path = normalize_existing_or_absolute(path)?;
        if self.overlays.remove(&path).is_none() {
            return Ok(RefreshOutcome::Unchanged);
        }
        if path.is_file() && self.included(&path) && supported(&path) {
            self.parse_and_store(&path, None)?;
        } else {
            self.remove_path(&path);
        }
        self.invalidate();
        Ok(RefreshOutcome::Updated)
    }

    pub fn refresh_path(&mut self, path: &Path) -> Result<RefreshOutcome> {
        let path = normalize_existing_or_absolute(path)?;
        if self.requires_full_reload(&path) {
            return Ok(RefreshOutcome::FullReloadRequired);
        }
        if self
            .overlays
            .keys()
            .any(|candidate| paths_equal(candidate, &path))
        {
            return Ok(RefreshOutcome::Unchanged);
        }
        if path.is_file() && self.included(&path) && supported(&path) {
            self.parse_and_store(&path, None)?;
            self.invalidate();
            return Ok(RefreshOutcome::Updated);
        }
        if self.remove_path(&path) {
            self.invalidate();
            return Ok(RefreshOutcome::Updated);
        }
        Ok(RefreshOutcome::Unchanged)
    }

    pub fn requires_full_reload(&self, path: &Path) -> bool {
        let path = normalize_existing_or_absolute(path).unwrap_or_else(|_| path.to_path_buf());
        paths_equal(&path, &self.config_path)
            || paths_equal(&path, &self.config.tsconfig)
            || self.resolver.uses_config(&path)
    }

    pub fn reload(&mut self) -> Result<()> {
        let overlays = std::mem::take(&mut self.overlays);
        let mut replacement = Self::load(self.workspace_root.clone(), self.config_path.clone())?;
        for (path, text) in overlays {
            replacement.update_document(path, text)?;
        }
        *self = replacement;
        Ok(())
    }

    pub fn diagnostics(&mut self) -> Result<RunResult> {
        if let Some(result) = &self.cached_result {
            return Ok(result.clone());
        }
        let analysis = analysis::analyze(
            &self.config,
            self.stylesheets.values().cloned().collect(),
            self.modules.values().cloned().collect(),
        );
        let mut diagnostics = analysis.diagnostics;
        diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        let has_errors = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity.is_error());
        let result = RunResult {
            diagnostics,
            unused_symbols: analysis.unused_symbols,
            has_errors,
        };
        self.cached_result = Some(result.clone());
        #[cfg(test)]
        {
            self.analysis_runs += 1;
        }
        Ok(result)
    }

    fn parse_and_store(&mut self, path: &Path, source: Option<&str>) -> Result<()> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("css" | "scss") => {
                let parsed = match source {
                    Some(source) => {
                        stylesheet::parse_source(path, source, self.config.ignore_exports)
                    }
                    None => stylesheet::parse(path, self.config.ignore_exports),
                }?;
                self.stylesheets.insert(path.to_path_buf(), parsed);
                self.modules.remove(path);
            }
            Some("ts" | "tsx") => {
                let parsed = match source {
                    Some(source) => typescript::parse_source(path, source, &self.resolver),
                    None => typescript::parse(path, &self.resolver),
                }?;
                self.modules.insert(path.to_path_buf(), parsed);
                self.stylesheets.remove(path);
            }
            _ => {}
        }
        Ok(())
    }

    fn included(&self, path: &Path) -> bool {
        path.starts_with(&self.config.source)
            && (!self
                .config
                .exclude
                .iter()
                .any(|folder| path.starts_with(folder))
                || self
                    .config
                    .include
                    .iter()
                    .any(|folder| path.starts_with(folder)))
    }

    fn remove_path(&mut self, path: &Path) -> bool {
        let stylesheet = self
            .stylesheets
            .keys()
            .find(|candidate| paths_equal(candidate, path))
            .cloned();
        let module = self
            .modules
            .keys()
            .find(|candidate| paths_equal(candidate, path))
            .cloned();
        stylesheet.is_some_and(|path| self.stylesheets.remove(&path).is_some())
            | module.is_some_and(|path| self.modules.remove(&path).is_some())
    }

    fn invalidate(&mut self) {
        self.cached_result = None;
    }

    #[cfg(test)]
    fn analysis_runs(&self) -> usize {
        self.analysis_runs
    }
}

fn supported(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts" | "tsx" | "css" | "scss")
    )
}

fn normalize_existing_or_absolute(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("could not resolve {}", path.display()));
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && parent.exists()
    {
        return Ok(fs::canonicalize(parent)?.join(name));
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        let left = left.to_string_lossy();
        let right = right.to_string_lossy();
        left.trim_start_matches(r"\\?\")
            .eq_ignore_ascii_case(right.trim_start_matches(r"\\?\"))
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, WorkspaceIndex) {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();
        let style = src.join("card.module.css");
        let code = src.join("card.ts");
        fs::write(&style, ".root { color: inherit; }").unwrap();
        fs::write(
            &code,
            "import styles from './card.module.css'; styles.root;",
        )
        .unwrap();
        let config = temp.path().join("style-contract.json");
        fs::write(&config, r#"{ convention: "camel-case" }"#).unwrap();
        let index = WorkspaceIndex::load(temp.path().to_path_buf(), config).unwrap();
        (temp, style, code, index)
    }

    #[test]
    fn overlay_changes_and_close_restores_disk_diagnostics() {
        let (_temp, _style, code, mut index) = fixture();
        assert!(index.diagnostics().unwrap().diagnostics.is_empty());
        index
            .update_document(
                code.clone(),
                "import styles from './card.module.css'; styles.missing;".into(),
            )
            .unwrap();
        assert!(
            index
                .diagnostics()
                .unwrap()
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "missing-symbol")
        );
        index.close_document(&code).unwrap();
        assert!(index.diagnostics().unwrap().diagnostics.is_empty());
    }

    #[test]
    fn repeated_diagnostics_reuses_the_cached_result() {
        let (_temp, _style, _code, mut index) = fixture();
        index.diagnostics().unwrap();
        index.diagnostics().unwrap();
        assert_eq!(index.analysis_runs(), 1);
    }

    #[test]
    fn invalid_overlay_keeps_the_last_valid_index_entry() {
        let (_temp, _style, code, mut index) = fixture();
        assert!(index.update_document(code, "import {".into()).is_err());
        assert!(index.diagnostics().unwrap().diagnostics.is_empty());
    }

    #[test]
    fn stylesheet_overlay_updates_cross_file_diagnostics() {
        let (_temp, style, _code, mut index) = fixture();
        index
            .update_document(style, ".renamed { color: inherit; }".into())
            .unwrap();
        let diagnostics = index.diagnostics().unwrap().diagnostics;
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule == "missing-symbol" && diagnostic.message.contains("root")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule == "unused-class" && diagnostic.message.contains("renamed")
        }));
    }

    #[test]
    fn identical_overlay_does_not_invalidate_cached_diagnostics() {
        let (_temp, _style, code, mut index) = fixture();
        let source = "import styles from './card.module.css'; styles.root;".to_owned();
        index.update_document(code.clone(), source.clone()).unwrap();
        index.diagnostics().unwrap();
        assert_eq!(index.analysis_runs(), 1);
        assert_eq!(
            index.update_document(code, source).unwrap(),
            RefreshOutcome::Unchanged
        );
        index.diagnostics().unwrap();
        assert_eq!(index.analysis_runs(), 1);
    }

    #[test]
    fn refreshes_created_and_deleted_files_without_reloading() {
        let (temp, _style, _code, mut index) = fixture();
        let added = temp.path().join("src/added.module.css");
        fs::write(&added, ".unused { color: inherit; }").unwrap();
        assert_eq!(index.refresh_path(&added).unwrap(), RefreshOutcome::Updated);
        let canonical = fs::canonicalize(&added).unwrap();
        assert!(
            index
                .diagnostics()
                .unwrap()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.rule == "unused-class" && diagnostic.location.path == canonical
                })
        );
        fs::remove_file(&added).unwrap();
        assert_eq!(index.refresh_path(&added).unwrap(), RefreshOutcome::Updated);
        assert!(
            index
                .diagnostics()
                .unwrap()
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.location.path != added)
        );
    }

    #[test]
    fn reports_every_unused_declaration_occurrence_but_one_diagnostic() {
        let (_temp, style, _code, mut index) = fixture();
        index
            .update_document(
                style,
                ".title::placeholder,\n.title:focus::placeholder { color: inherit; }".into(),
            )
            .unwrap();
        let result = index.diagnostics().unwrap();
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule == "unused-class")
                .count(),
            1
        );
        assert_eq!(result.unused_symbols.len(), 2);
        assert_eq!(result.unused_symbols[0].location.line, 1);
        assert_eq!(result.unused_symbols[1].location.line, 2);
    }

    #[test]
    fn disabled_unused_rule_does_not_report_fading_metadata() {
        let (temp, style, _code, _index) = fixture();
        fs::write(
            &style,
            ".unused { color: inherit; }\n.unused:hover { color: inherit; }",
        )
        .unwrap();
        let config = temp.path().join("style-contract.json");
        fs::write(
            &config,
            r#"{ convention: "camel-case", rules: { "unused-class": "off" } }"#,
        )
        .unwrap();
        let mut index = WorkspaceIndex::load(temp.path().to_path_buf(), config).unwrap();
        let result = index.diagnostics().unwrap();
        assert!(result.unused_symbols.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != "unused-class")
        );
    }
}
