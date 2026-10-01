use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    RunResult, analysis, config::Config, diagnostic::Diagnostic, discovery, resolver::Resolver,
    stylesheet, typescript,
};

/// A StyleContract workspace with editor-owned text overlays.
///
/// The command-line runner and editor runner share the same parsers and analyzer;
/// overlays only replace the file-reading boundary.
pub struct WorkspaceIndex {
    config: Config,
    overlays: BTreeMap<PathBuf, String>,
}

impl WorkspaceIndex {
    pub fn load(workspace_root: PathBuf, config_path: PathBuf) -> Result<Self> {
        Ok(Self {
            config: Config::from_path(workspace_root, config_path)?,
            overlays: BTreeMap::new(),
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn update_document(&mut self, path: PathBuf, text: String) -> Result<()> {
        self.overlays.insert(normalize(&path)?, text);
        Ok(())
    }

    pub fn close_document(&mut self, path: &Path) {
        if let Ok(path) = normalize(path) {
            self.overlays.remove(&path);
        }
    }

    pub fn reload(&mut self, workspace_root: PathBuf, config_path: PathBuf) -> Result<()> {
        self.config = Config::from_path(workspace_root, config_path)?;
        Ok(())
    }

    pub fn diagnostics(&self) -> Result<RunResult> {
        let files = discovery::discover(&self.config)?;
        let resolver = Resolver::load(&self.config.tsconfig, &self.config.source)?;
        let styles = files
            .stylesheets
            .iter()
            .map(|path| match self.overlays.get(path) {
                Some(source) => stylesheet::parse_source(path, source, self.config.ignore_exports),
                None => stylesheet::parse(path, self.config.ignore_exports),
            })
            .collect::<Result<Vec<_>>>()?;
        let modules = files
            .typescript
            .iter()
            .map(|path| match self.overlays.get(path) {
                Some(source) => typescript::parse_source(path, source, &resolver),
                None => typescript::parse(path, &resolver),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut diagnostics: Vec<Diagnostic> = analysis::analyze(&self.config, styles, modules);
        diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        let has_errors = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity.is_error());
        Ok(RunResult {
            diagnostics,
            has_errors,
        })
    }
}

fn normalize(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("could not resolve {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_changes_and_close_restores_disk_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        fs::create_dir(&src).unwrap();
        let style = src.join("card.module.css");
        let code = src.join("card.ts");
        fs::write(&style, ".root {}").unwrap();
        fs::write(
            &code,
            "import styles from './card.module.css'; styles.root;",
        )
        .unwrap();
        fs::write(
            temp.path().join("style-contract.json"),
            r#"{ convention: "camel-case" }"#,
        )
        .unwrap();
        let mut index = WorkspaceIndex::load(
            temp.path().to_path_buf(),
            temp.path().join("style-contract.json"),
        )
        .unwrap();
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
                .any(|d| d.rule == "missing-symbol")
        );
        index.close_document(&code);
        assert!(index.diagnostics().unwrap().diagnostics.is_empty());
    }
}
