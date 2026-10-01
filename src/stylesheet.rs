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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassScope {
    Local,
    Global,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub kind: SymbolKind,
    pub scope: ClassScope,
    pub location: Location,
}

#[derive(Debug)]
pub struct Stylesheet {
    pub path: PathBuf,
    pub declarations: Vec<Declaration>,
    pub references: Vec<StylesheetReference>,
    pub dynamic_locations: Vec<Location>,
}

#[derive(Debug, Clone)]
pub struct StylesheetReference {
    pub name: String,
    pub stylesheet: PathBuf,
    pub scope: ClassScope,
    pub location: Location,
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
    let _tree = parser.parse(&source, None).ok_or_else(|| {
        anyhow!(
            "SCSS parser did not return a syntax tree for {}",
            path.display()
        )
    })?;
    let mut masked = mask_comments_and_strings(&source);
    validate_structure(path, &masked)?;
    let default_scope = if is_module_stylesheet(path) {
        ClassScope::Local
    } else {
        ClassScope::Global
    };
    let scope_ranges = extract_scope_ranges(&masked);
    let (references, reference_class_offsets) =
        extract_stylesheet_references(path, &source, &masked, default_scope, &scope_ranges);
    let mut declarations = Vec::new();
    if !ignore_exports {
        extract_exports(path, &source, &mut masked, &mut declarations);
    } else {
        mask_export_blocks(&mut masked);
    }

    let class_re = Regex::new(r"\.(-?[A-Za-z_][A-Za-z0-9_-]*)").expect("valid regex");
    for capture in class_re.captures_iter(&masked) {
        let matched = capture.get(1).expect("capture exists");
        let dot = matched.start() - 1;
        if reference_class_offsets.contains(&dot) {
            continue;
        }
        declarations.push(Declaration {
            name: matched.as_str().to_owned(),
            kind: SymbolKind::Class,
            scope: scope_at(matched.start() - 1, default_scope, &scope_ranges),
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
            && left.scope == right.scope
            && left.name == right.name
            && left.location.line == right.location.line
            && left.location.column == right.location.column
    });
    Ok(Stylesheet {
        path: path.to_path_buf(),
        declarations,
        references,
        dynamic_locations,
    })
}

fn extract_stylesheet_references(
    path: &std::path::Path,
    source: &str,
    masked: &str,
    default_scope: ClassScope,
    scope_ranges: &[ScopeRange],
) -> (Vec<StylesheetReference>, std::collections::BTreeSet<usize>) {
    let mut references = Vec::new();
    let mut class_offsets = std::collections::BTreeSet::new();
    let class_re = Regex::new(r"\.(-?[A-Za-z_][A-Za-z0-9_-]*)").expect("valid regex");
    let extend_re = Regex::new(r"@extend\s+([^;]+)").expect("valid regex");
    for extend in extend_re.captures_iter(masked) {
        let body = extend.get(1).expect("extend body matched");
        for class in class_re.captures_iter(body.as_str()) {
            let name = class.get(1).expect("class matched");
            let dot = body.start() + name.start() - 1;
            class_offsets.insert(dot);
            references.push(StylesheetReference {
                name: name.as_str().to_owned(),
                stylesheet: path.to_path_buf(),
                scope: scope_at(dot, default_scope, scope_ranges),
                location: location_at(path, source, body.start() + name.start()),
            });
        }
    }

    let comment_masked = mask_comments(source);
    let composes_re = Regex::new(r"(?m):?composes\s*:\s*([^;{}]+)").expect("valid regex");
    let name_re = Regex::new(r"[A-Za-z_][A-Za-z0-9_-]*").expect("valid regex");
    for composition in composes_re.captures_iter(&comment_masked) {
        let body = composition.get(1).expect("composition body matched");
        let (names, origin) = split_composition(body.as_str());
        let target = match origin {
            None => path.to_path_buf(),
            Some(value) if value.eq_ignore_ascii_case("global") => continue,
            Some(value) => {
                let Some(target) =
                    unquote(value).and_then(|specifier| resolve_composition_path(path, specifier))
                else {
                    continue;
                };
                target
            }
        };
        for name in name_re.find_iter(names) {
            references.push(StylesheetReference {
                name: name.as_str().to_owned(),
                stylesheet: target.clone(),
                scope: scope_at(body.start() + name.start(), default_scope, scope_ranges),
                location: location_at(path, source, body.start() + name.start()),
            });
        }
    }
    (references, class_offsets)
}

fn split_composition(value: &str) -> (&str, Option<&str>) {
    let from_re = Regex::new(r"(?i)\s+from\s+").expect("valid regex");
    if let Some(found) = from_re.find(value) {
        (&value[..found.start()], Some(value[found.end()..].trim()))
    } else {
        (value, None)
    }
}

fn unquote(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && matches!(bytes[0], b'\'' | b'"')
        && bytes.last().copied() == Some(bytes[0])
    {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

fn resolve_composition_path(path: &std::path::Path, specifier: &str) -> Option<PathBuf> {
    let candidate = path.parent()?.join(specifier);
    fs::canonicalize(candidate).ok()
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
                scope: ClassScope::Local,
                location: location_at(path, source, byte),
            });
        }
        masked.replace_range(
            export_match.start()..=close,
            &" ".repeat(close + 1 - export_match.start()),
        );
    }
}

fn is_module_stylesheet(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".module.css") || name.ends_with(".module.scss"))
}

#[derive(Debug)]
struct ScopeRange {
    start: usize,
    end: usize,
    scope: ClassScope,
}

fn extract_scope_ranges(source: &str) -> Vec<ScopeRange> {
    let scope_re = Regex::new(r":(global|local)\s*([({])").expect("valid regex");
    let mut ranges = Vec::new();
    for captures in scope_re.captures_iter(source) {
        let scope = match captures.get(1).expect("scope matched").as_str() {
            "global" => ClassScope::Global,
            "local" => ClassScope::Local,
            _ => unreachable!(),
        };
        let delimiter = captures.get(2).expect("delimiter matched");
        let open = delimiter.start();
        let close = if delimiter.as_str() == "{" {
            matching_delimiter(source, open, b'{', b'}')
        } else {
            matching_delimiter(source, open, b'(', b')')
        };
        if let Some(close) = close {
            ranges.push(ScopeRange {
                start: open + 1,
                end: close,
                scope,
            });
        }
    }
    ranges
}

fn scope_at(byte: usize, default: ClassScope, ranges: &[ScopeRange]) -> ClassScope {
    ranges
        .iter()
        .filter(|range| byte >= range.start && byte < range.end)
        .max_by_key(|range| range.start)
        .map_or(default, |range| range.scope)
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
    matching_delimiter(source, open, b'{', b'}')
}

fn matching_delimiter(source: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            if depth == 0 {
                return None;
            }
            depth -= 1;
            if depth == 0 {
                return Some(open + offset);
            }
        }
    }
    None
}

fn validate_structure(path: &std::path::Path, source: &str) -> Result<()> {
    let mut braces = Vec::new();
    for (byte, character) in source.char_indices() {
        match character {
            '{' => braces.push(byte),
            '}' if braces.pop().is_none() => {
                let location = location_at(path, source, byte);
                return Err(anyhow!(
                    "unmatched closing brace in {}:{}:{}",
                    path.display(),
                    location.line,
                    location.column
                ));
            }
            _ => {}
        }
    }
    if let Some(byte) = braces.pop() {
        let location = location_at(path, source, byte);
        return Err(anyhow!(
            "unmatched opening brace in {}:{}:{}",
            path.display(),
            location.line,
            location.column
        ));
    }
    Ok(())
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

fn mask_comments(source: &str) -> String {
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

    #[test]
    fn accepts_sass_module_directives_and_mixins() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.scss");
        fs::write(
            &path,
            "@use 'src/styles/variables' as *;\n@use 'src/styles/mixins' as *;\n.textField { p { @include p2; } }",
        )
        .unwrap();
        let parsed = parse(&path, false).unwrap();
        assert!(
            parsed
                .declarations
                .iter()
                .any(|declaration| declaration.name == "textField")
        );
    }

    #[test]
    fn rejects_unbalanced_stylesheets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("broken.scss");
        fs::write(&path, ".root { .child {}").unwrap();
        let error = parse(&path, false).unwrap_err();
        assert!(error.to_string().contains("unmatched opening brace"));
    }

    #[test]
    fn classifies_module_scopes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.module.scss");
        fs::write(
            &path,
            ".local {} :global(.Mui-root) {} :global { .vendor { :local(.nested) {} } }",
        )
        .unwrap();
        let parsed = parse(&path, false).unwrap();
        let scopes: std::collections::BTreeMap<_, _> = parsed
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == SymbolKind::Class)
            .map(|declaration| (declaration.name.as_str(), declaration.scope))
            .collect();
        assert_eq!(scopes["local"], ClassScope::Local);
        assert_eq!(scopes["Mui-root"], ClassScope::Global);
        assert_eq!(scopes["vendor"], ClassScope::Global);
        assert_eq!(scopes["nested"], ClassScope::Local);
    }

    #[test]
    fn non_module_stylesheets_default_to_global() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("global.scss");
        fs::write(&path, ".vendor {} :local(.optIn) {}").unwrap();
        let parsed = parse(&path, false).unwrap();
        assert_eq!(parsed.declarations[0].scope, ClassScope::Global);
        assert_eq!(parsed.declarations[1].scope, ClassScope::Local);
    }

    #[test]
    fn extracts_extend_and_composition_as_references_not_declarations() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.module.scss");
        fs::write(
            &path,
            ".base {} .extended { @extend .base; } .composed { composes: base extended; }",
        )
        .unwrap();
        let parsed = parse(&path, false).unwrap();
        let declarations: Vec<_> = parsed
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect();
        assert_eq!(declarations, vec!["base", "extended", "composed"]);
        let references: Vec<_> = parsed
            .references
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        assert_eq!(references, vec!["base", "base", "extended"]);
    }
}
