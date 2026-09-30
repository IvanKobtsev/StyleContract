use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use rayon::prelude::*;
use regex::Regex;
use tree_sitter::Parser;

use crate::diagnostic::{Location, location_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Class,
    Export,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub kind: SymbolKind,
    pub location: Location,
}

#[derive(Debug)]
pub struct Stylesheet {
    pub path: PathBuf,
    pub declarations: Vec<Declaration>,
    pub dynamic_locations: Vec<Location>,
}

pub fn parse_all(paths: &[PathBuf], ignore_exports: bool) -> Result<Vec<Stylesheet>> {
    paths
        .par_iter()
        .map(|path| parse(path, ignore_exports))
        .collect()
}

pub fn parse(path: &std::path::Path, ignore_exports: bool) -> Result<Stylesheet> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("could not read stylesheet {}", path.display()))?;
    let mut parser = Parser::new();
    parser
        .set_language(&arborium_scss::language().into())
        .map_err(|error| anyhow!("could not load SCSS parser: {error}"))?;
    let tree = parser.parse(&source, None).ok_or_else(|| {
        anyhow!(
            "SCSS parser did not return a syntax tree for {}",
            path.display()
        )
    })?;
    if tree.root_node().has_error() {
        return Err(anyhow!("could not parse stylesheet {}", path.display()));
    }

    let mut masked = mask_comments_and_strings(&source);
    let mut declarations = Vec::new();
    if !ignore_exports {
        extract_exports(path, &source, &mut masked, &mut declarations);
    } else {
        mask_export_blocks(&mut masked);
    }

    let class_re = Regex::new(r"\.(-?[A-Za-z_][A-Za-z0-9_-]*)").expect("valid regex");
    for capture in class_re.captures_iter(&masked) {
        let matched = capture.get(1).expect("capture exists");
        declarations.push(Declaration {
            name: matched.as_str().to_owned(),
            kind: SymbolKind::Class,
            location: location_at(path, &source, matched.start()),
        });
    }

    let interpolation_re = Regex::new(r"(?:\.\s*#\{|#\{[^}]+\})").expect("valid regex");
    let dynamic_locations = interpolation_re
        .find_iter(&masked)
        .map(|matched| location_at(path, &source, matched.start()))
        .collect();

    declarations.sort_by_key(|declaration| {
        (
            declaration.location.line,
            declaration.location.column,
            declaration.name.clone(),
        )
    });
    declarations.dedup_by(|left, right| {
        left.kind == right.kind
            && left.name == right.name
            && left.location.line == right.location.line
            && left.location.column == right.location.column
    });
    Ok(Stylesheet {
        path: path.to_path_buf(),
        declarations,
        dynamic_locations,
    })
}

fn extract_exports(
    path: &std::path::Path,
    source: &str,
    masked: &mut String,
    declarations: &mut Vec<Declaration>,
) {
    let export_re = Regex::new(r":export\s*\{").expect("valid regex");
    let key_re = Regex::new(r"(?m)([A-Za-z_][A-Za-z0-9_-]*)\s*:").expect("valid regex");
    let snapshot = masked.to_owned();
    for export_match in export_re.find_iter(&snapshot) {
        let open = export_match.end() - 1;
        let Some(close) = matching_brace(&snapshot, open) else {
            continue;
        };
        let body = &snapshot[open + 1..close];
        for capture in key_re.captures_iter(body) {
            let matched = capture.get(1).expect("capture exists");
            let byte = open + 1 + matched.start();
            declarations.push(Declaration {
                name: matched.as_str().to_owned(),
                kind: SymbolKind::Export,
                location: location_at(path, source, byte),
            });
        }
        masked.replace_range(
            export_match.start()..=close,
            &" ".repeat(close + 1 - export_match.start()),
        );
    }
}

fn mask_export_blocks(masked: &mut String) {
    let export_re = Regex::new(r":export\s*\{").expect("valid regex");
    let snapshot = masked.to_owned();
    for export_match in export_re.find_iter(&snapshot) {
        let open = export_match.end() - 1;
        if let Some(close) = matching_brace(&snapshot, open) {
            masked.replace_range(
                export_match.start()..=close,
                &" ".repeat(close + 1 - export_match.start()),
            );
        }
    }
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn mask_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            let start = index;
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            blank_preserving_newlines(&mut output[start..index]);
        } else if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'/' {
            let start = index;
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            blank_preserving_newlines(&mut output[start..index]);
        } else if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            let start = index;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            blank_preserving_newlines(&mut output[start..index]);
        } else {
            index += 1;
        }
    }
    String::from_utf8(output).expect("mask retains valid UTF-8")
}

fn blank_preserving_newlines(bytes: &mut [u8]) {
    for byte in bytes {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_classes_exports_and_dynamic_selectors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.scss");
        fs::write(
            &path,
            ".root, .other { &.active {} }\n:export { brand-color: red; }\n.#{$name} {}",
        )
        .unwrap();
        let parsed = parse(&path, false).unwrap();
        let names: Vec<_> = parsed
            .declarations
            .iter()
            .map(|declaration| (&declaration.name, declaration.kind))
            .collect();
        assert!(names.contains(&(&"root".to_owned(), SymbolKind::Class)));
        assert!(names.contains(&(&"active".to_owned(), SymbolKind::Class)));
        assert!(names.contains(&(&"brand-color".to_owned(), SymbolKind::Export)));
        assert_eq!(parsed.dynamic_locations.len(), 1);
    }
}
