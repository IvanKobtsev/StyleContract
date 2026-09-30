use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::Config;

#[derive(Debug, Default)]
pub struct Files {
    pub typescript: Vec<PathBuf>,
    pub stylesheets: Vec<PathBuf>,
}

pub fn discover(config: &Config) -> Result<Files> {
    let mut files = Files::default();
    for entry in WalkDir::new(&config.source).follow_links(false) {
        let entry = entry.context("could not traverse source tree")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = fs::canonicalize(entry.path())
            .with_context(|| format!("could not resolve {}", entry.path().display()))?;
        if excluded(&path, config) {
            continue;
        }
        match path.extension().and_then(|value| value.to_str()) {
            Some("ts" | "tsx") => files.typescript.push(path),
            Some("css" | "scss") => files.stylesheets.push(path),
            _ => {}
        }
    }
    files.typescript.sort();
    files.stylesheets.sort();
    Ok(files)
}

fn excluded(path: &std::path::Path, config: &Config) -> bool {
    let is_excluded = config.exclude.iter().any(|folder| path.starts_with(folder));
    let explicitly_included = config.include.iter().any(|folder| path.starts_with(folder));
    is_excluded && !explicitly_included
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Cli, naming::Convention};

    #[test]
    fn include_overrides_exclude() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let excluded_dir = src.join("generated");
        let included_dir = excluded_dir.join("keep");
        fs::create_dir_all(&included_dir).unwrap();
        fs::write(excluded_dir.join("skip.ts"), "").unwrap();
        fs::write(included_dir.join("keep.ts"), "").unwrap();
        let config = Config::from_cli(
            Cli {
                convention: Convention::CamelCase,
                source: src,
                exclude: vec![excluded_dir],
                include: vec![included_dir],
                ignore_exports: false,
                rule: vec![],
                tsconfig: temp.path().join("tsconfig.json"),
            },
            temp.path().to_path_buf(),
        )
        .unwrap();
        let files = discover(&config).unwrap();
        assert_eq!(files.typescript.len(), 1);
        assert!(files.typescript[0].ends_with("keep.ts"));
    }
}
