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

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub path: PathBuf,
    pub declarations: Vec<Declaration>,
    pub dependent_declarations: Vec<DependentDeclaration>,
    pub references: Vec<StylesheetReference>,
    pub dynamic_locations: Vec<Location>,
    pub empty_rules: Vec<Location>,
    pub imports: Vec<StylesheetImport>,
}

#[derive(Debug, Clone)]
pub struct DependentDeclaration {
    pub name: String,
    pub prerequisite_paths: Vec<Vec<String>>,
    pub location: Location,
}

#[derive(Debug, Clone)]
pub struct StylesheetImport {
    pub specifier: String,
    pub location: Location,
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
    parse_source(path, &source, ignore_exports)
}

/// Parse stylesheet text supplied by an editor instead of reading from disk.
pub fn parse_source(
    path: &std::path::Path,
    source: &str,
    ignore_exports: bool,
) -> Result<Stylesheet> {
    let mut parser = Parser::new();
    parser
        .set_language(&arborium_scss::language().into())
        .map_err(|error| anyhow!("could not load SCSS parser: {error}"))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        anyhow!(
            "SCSS parser did not return a syntax tree for {}",
            path.display()
        )
    })?;
    let mut masked = mask_comments_and_strings(source);
    validate_structure(path, &masked)?;
    let empty_rules = extract_empty_rules(path, source, tree.root_node());
    let imports = extract_imports(path, source, tree.root_node());
    let default_scope = if is_module_stylesheet(path) {
        ClassScope::Local
    } else {
        ClassScope::Global
    };
    let scope_ranges = extract_scope_ranges(&masked);
    let dependent_declarations = extract_dependent_declarations(
        path,
        source,
        tree.root_node(),
        default_scope,
        &scope_ranges,
    );
    let (references, reference_class_offsets) =
        extract_stylesheet_references(path, source, &masked, default_scope, &scope_ranges);
    let mut declarations = Vec::new();
    if !ignore_exports {
        extract_exports(path, source, &mut masked, &mut declarations);
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
            location: location_at(path, source, matched.start()),
        });
    }

    let interpolation_re = Regex::new(r"(?:\.\s*#\{|#\{[^}]+\})").expect("valid regex");
    let dynamic_locations = interpolation_re
        .find_iter(&masked)
        .map(|matched| location_at(path, source, matched.start()))
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
        dependent_declarations,
        references,
        dynamic_locations,
        empty_rules,
        imports,
    })
}

fn extract_dependent_declarations(
    path: &std::path::Path,
    source: &str,
    root: tree_sitter::Node<'_>,
    default_scope: ClassScope,
    scope_ranges: &[ScopeRange],
) -> Vec<DependentDeclaration> {
    fn visit(
        path: &std::path::Path,
        source: &str,
        node: tree_sitter::Node<'_>,
        parent_paths: &[Vec<String>],
        default_scope: ClassScope,
        scope_ranges: &[ScopeRange],
        output: &mut Vec<DependentDeclaration>,
    ) {
        let mut child_parent_paths = Vec::new();
        if node.kind() == "rule_set" && !node.has_error() {
            let mut cursor = node.walk();
            if let Some(selectors) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "selectors")
            {
                let selector_text = &source[selectors.byte_range()];
                let branches = split_selector_branches(selector_text);
                for (branch_offset, branch) in branches {
                    let trimmed = branch.trim();
                    let leading = branch.len() - branch.trim_start().len();
                    let branch_start = selectors.start_byte() + branch_offset + leading;
                    if let Some(suffix) = trimmed.strip_prefix('&') {
                        if same_compound_suffix(suffix) {
                            let suffix_classes = classes_with_offsets(suffix);
                            for parent in parent_paths {
                                let mut resolved = parent.clone();
                                for (name, relative) in &suffix_classes {
                                    let is_local = scope_at(
                                        branch_start + 1 + relative,
                                        default_scope,
                                        scope_ranges,
                                    ) == ClassScope::Local;
                                    if !resolved.is_empty() && is_local {
                                        push_dependent(
                                            output,
                                            name,
                                            resolved.clone(),
                                            location_at(path, source, branch_start + 1 + relative),
                                        );
                                    }
                                    if is_local {
                                        resolved.push(name.clone());
                                    }
                                }
                                child_parent_paths.push(resolved);
                            }
                        }
                    } else {
                        for (compound_offset, compound) in selector_compounds(trimmed) {
                            let classes = classes_with_offsets(compound);
                            let mut prior = Vec::new();
                            for (name, relative) in classes {
                                let absolute = branch_start + compound_offset + relative;
                                let is_local = scope_at(absolute, default_scope, scope_ranges)
                                    == ClassScope::Local;
                                if !prior.is_empty() && is_local {
                                    push_dependent(
                                        output,
                                        &name,
                                        prior.clone(),
                                        location_at(path, source, absolute),
                                    );
                                }
                                if is_local {
                                    prior.push(name);
                                }
                            }
                            if !prior.is_empty() {
                                child_parent_paths.push(prior);
                            }
                        }
                    }
                }
            }
        }

        let inherited = if child_parent_paths.is_empty() {
            parent_paths
        } else {
            &child_parent_paths
        };
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if node.kind() != "rule_set" || child.kind() == "block" {
                visit(
                    path,
                    source,
                    child,
                    inherited,
                    default_scope,
                    scope_ranges,
                    output,
                );
            }
        }
    }

    let mut output = Vec::new();
    visit(
        path,
        source,
        root,
        &[],
        default_scope,
        scope_ranges,
        &mut output,
    );
    output.sort_by_key(|item| (item.location.line, item.location.column, item.name.clone()));
    output
}

fn push_dependent(
    output: &mut Vec<DependentDeclaration>,
    name: &str,
    prerequisites: Vec<String>,
    location: Location,
) {
    if let Some(existing) = output.iter_mut().find(|item| {
        item.name == name
            && item.location.line == location.line
            && item.location.column == location.column
    }) {
        if !existing.prerequisite_paths.contains(&prerequisites) {
            existing.prerequisite_paths.push(prerequisites);
        }
    } else {
        output.push(DependentDeclaration {
            name: name.to_owned(),
            prerequisite_paths: vec![prerequisites],
            location,
        });
    }
}

fn split_selector_branches(selector: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (offset, character) in selector.char_indices() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push((start, &selector[start..offset]));
                start = offset + 1;
            }
            _ => {}
        }
    }
    result.push((start, &selector[start..]));
    result
}

fn same_compound_suffix(selector: &str) -> bool {
    let mut depth = 0usize;
    for character in selector.chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ' ' | '\t' | '\r' | '\n' | '>' | '+' | '~' if depth == 0 => return false,
            _ => {}
        }
    }
    true
}

fn selector_compounds(selector: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (offset, character) in selector.char_indices() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ' ' | '\t' | '\r' | '\n' | '>' | '+' | '~' if depth == 0 => {
                if start < offset {
                    result.push((start, &selector[start..offset]));
                }
                start = offset + character.len_utf8();
            }
            _ => {}
        }
    }
    if start < selector.len() {
        result.push((start, &selector[start..]));
    }
    result
}

fn classes_with_offsets(selector: &str) -> Vec<(String, usize)> {
    let class_re = Regex::new(r"\.(-?[A-Za-z_][A-Za-z0-9_-]*)").expect("valid regex");
    class_re
        .captures_iter(selector)
        .map(|capture| {
            let name = capture.get(1).expect("class matched");
            (name.as_str().to_owned(), name.start())
        })
        .collect()
}

fn extract_imports(
    path: &std::path::Path,
    source: &str,
    root: tree_sitter::Node<'_>,
) -> Vec<StylesheetImport> {
    fn visit(
        path: &std::path::Path,
        source: &str,
        node: tree_sitter::Node<'_>,
        imports: &mut Vec<StylesheetImport>,
    ) {
        if matches!(
            node.kind(),
            "use_statement" | "forward_statement" | "import_statement"
        ) {
            let statement = &source[node.byte_range()];
            let string_re = Regex::new(r#"["']([^"']+)["']"#).expect("valid regex");
            for capture in string_re.captures_iter(statement) {
                let specifier = capture.get(1).expect("specifier matched");
                imports.push(StylesheetImport {
                    specifier: specifier.as_str().to_owned(),
                    location: location_at(path, source, node.start_byte() + specifier.start()),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(path, source, child, imports);
        }
    }

    let mut imports = Vec::new();
    visit(path, source, root, &mut imports);
    imports.sort_by_key(|item| (item.location.line, item.location.column));
    imports
}

fn extract_empty_rules(
    path: &std::path::Path,
    source: &str,
    root: tree_sitter::Node<'_>,
) -> Vec<Location> {
    fn visit(
        path: &std::path::Path,
        source: &str,
        node: tree_sitter::Node<'_>,
        locations: &mut Vec<Location>,
    ) {
        if node.kind() == "rule_set" && !node.has_error() {
            let mut cursor = node.walk();
            let mut selectors = None;
            let mut block = None;
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "selectors" => selectors = Some(child),
                    "block" => block = Some(child),
                    _ => {}
                }
            }
            if let (Some(selectors), Some(block)) = (selectors, block) {
                let selector = &source[selectors.byte_range()];
                let is_export = selector.trim() == ":export";
                let mut block_cursor = block.walk();
                let has_content = block
                    .named_children(&mut block_cursor)
                    .any(|child| child.kind() != "comment");
                if !is_export && !has_content {
                    let leading = selector.len() - selector.trim_start().len();
                    locations.push(location_at(path, source, selectors.start_byte() + leading));
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(path, source, child, locations);
        }
    }

    let mut locations = Vec::new();
    visit(path, source, root, &mut locations);
    locations.sort_by_key(|location| (location.line, location.column));
    locations
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
    fn extracts_nested_and_flat_dependent_class_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.module.scss");
        fs::write(
            &path,
            ".root, .other { &.active { &.busy {} } }\n.card.selected:hover {}\n.parent .child.highlighted {}",
        )
        .unwrap();
        let parsed = parse(&path, false).unwrap();

        let active = parsed
            .dependent_declarations
            .iter()
            .find(|declaration| declaration.name == "active")
            .unwrap();
        assert_eq!(
            active.prerequisite_paths,
            vec![vec!["root".to_owned()], vec!["other".to_owned()]]
        );
        let busy = parsed
            .dependent_declarations
            .iter()
            .find(|declaration| declaration.name == "busy")
            .unwrap();
        assert_eq!(
            busy.prerequisite_paths,
            vec![
                vec!["root".to_owned(), "active".to_owned()],
                vec!["other".to_owned(), "active".to_owned()],
            ]
        );
        assert!(parsed.dependent_declarations.iter().any(|declaration| {
            declaration.name == "selected"
                && declaration.prerequisite_paths == vec![vec!["card".to_owned()]]
        }));
        assert!(parsed.dependent_declarations.iter().any(|declaration| {
            declaration.name == "highlighted"
                && declaration.prerequisite_paths == vec![vec!["child".to_owned()]]
        }));
        assert!(!parsed.dependent_declarations.iter().any(|declaration| {
            declaration.name == "child"
                && declaration
                    .prerequisite_paths
                    .iter()
                    .flatten()
                    .any(|name| name == "parent")
        }));
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

    #[test]
    fn extracts_only_semantically_empty_selector_rules() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.module.scss");
        let source = r#".empty {}
.commentOnly { /* explanation */ }
.parent { .nestedEmpty {} }
button,
input:hover { }
.property { color: red; }
.customProperty { --color: red; }
.variable { $color: red; }
.include { @include example; }
:export {}
@media (min-width: 1px) {}
@supports (display: grid) { .inside { display: grid; } }
"#;
        fs::write(&path, source).unwrap();
        let parsed = parse(&path, false).unwrap();
        let lines: Vec<_> = parsed
            .empty_rules
            .iter()
            .map(|location| location.line)
            .collect();
        assert_eq!(lines, vec![1, 2, 3, 4]);
        assert_eq!(parsed.empty_rules[3].column, 1);
    }

    #[test]
    fn extracts_sass_stylesheet_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styles.module.scss");
        let source = "@use \"./base.module.scss\" as *;\n@forward './tokens';\n@import \"./first\", \"./second.module.scss\";";
        fs::write(&path, source).unwrap();
        let parsed = parse(&path, false).unwrap();
        let imports: Vec<_> = parsed
            .imports
            .iter()
            .map(|item| (item.specifier.as_str(), item.location.line))
            .collect();
        assert_eq!(
            imports,
            vec![
                ("./base.module.scss", 1),
                ("./tokens", 2),
                ("./first", 3),
                ("./second.module.scss", 3),
            ]
        );
    }
}
