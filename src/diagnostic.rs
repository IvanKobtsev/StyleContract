use std::{collections::BTreeMap, fmt, path::PathBuf};

use anyhow::{Result, anyhow};
use tree_sitter::Node;

use crate::display_path;

pub const RULES: [&str; 9] = [
    "missing-symbol",
    "unused-class",
    "unused-dependent-class",
    "unused-export",
    "naming-convention-local",
    "naming-convention-global",
    "dynamic-reference",
    "empty-rule",
    "module-to-module-import",
];
pub const LEGACY_NAMING_RULE: &str = "naming-convention";
pub const LEGACY_RULE_ALIASES: [(&str, &str); 4] = [
    ("no-missing-symbols", "missing-symbol"),
    ("no-unused-classes", "unused-class"),
    ("no-unused-exports", "unused-export"),
    ("no-dynamic-references", "dynamic-reference"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Off,
}

impl Severity {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn is_error(self) -> bool {
        self == Self::Error
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Off => write!(f, "off"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct Suppression {
    pub start_line: usize,
    pub end_line: usize,
    pub rules: Vec<String>,
}

impl Suppression {
    pub fn suppresses(&self, line: usize, rule: &str) -> bool {
        self.start_line <= line
            && line <= self.end_line
            && (self.rules.is_empty() || self.rules.iter().any(|item| item == rule))
    }
}

pub fn parse_suppressions(
    path: &std::path::Path,
    source: &str,
    root: Node<'_>,
) -> Result<Vec<Suppression>> {
    fn collect_comments(node: Node<'_>, output: &mut Vec<std::ops::Range<usize>>) {
        if node.kind() == "comment" {
            output.push(node.byte_range());
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_comments(child, output);
        }
    }

    let mut comments = Vec::new();
    collect_comments(root, &mut comments);
    let mut suppressions = Vec::new();
    let mut blocks: Vec<(usize, Vec<String>)> = Vec::new();
    for comment in comments {
        for (line_offset, comment_line) in source[comment.clone()].lines().enumerate() {
            let Some(marker) = comment_line.find("@sc-ignore") else {
                continue;
            };
            let line = source[..comment.start].lines().count() + line_offset + 1;
            let directive = comment_line[marker..].trim_end_matches("*/").trim_end();
            let mut parts = directive.splitn(2, char::is_whitespace);
            let name = parts.next().unwrap_or_default();
            let arguments = parts.next().unwrap_or_default().trim();
            match name {
                "@sc-ignore" => suppressions.push(Suppression {
                    start_line: line + 1,
                    end_line: line + 1,
                    rules: parse_suppression_rules(path, line, arguments)?,
                }),
                "@sc-ignore-start" => {
                    blocks.push((line, parse_suppression_rules(path, line, arguments)?));
                }
                "@sc-ignore-end" => {
                    if !arguments.is_empty() {
                        return Err(suppression_error(
                            path,
                            line,
                            "@sc-ignore-end does not accept rule names",
                        ));
                    }
                    let Some((start_line, rules)) = blocks.pop() else {
                        return Err(suppression_error(path, line, "unmatched @sc-ignore-end"));
                    };
                    suppressions.push(Suppression {
                        start_line: start_line + 1,
                        end_line: line.saturating_sub(1),
                        rules,
                    });
                }
                _ => {
                    return Err(suppression_error(
                        path,
                        line,
                        format!("unknown suppression directive '{name}'"),
                    ));
                }
            }
        }
    }
    if let Some((line, _)) = blocks.last() {
        return Err(suppression_error(path, *line, "unmatched @sc-ignore-start"));
    }
    Ok(suppressions)
}

fn parse_suppression_rules(
    path: &std::path::Path,
    line: usize,
    value: &str,
) -> Result<Vec<String>> {
    let rules = value
        .split(|character: char| character == ',' || character.is_whitespace())
        .filter(|rule| !rule.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Some(rule) = rules.iter().find(|rule| !RULES.contains(&rule.as_str())) {
        return Err(suppression_error(
            path,
            line,
            format!("unknown StyleContract rule '{rule}'"),
        ));
    }
    Ok(rules)
}

fn suppression_error(
    path: &std::path::Path,
    line: usize,
    message: impl std::fmt::Display,
) -> anyhow::Error {
    anyhow!("{}:{line}:1 {message}", path.display())
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub location: Location,
    pub severity: Severity,
    pub rule: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnusedSymbolKind {
    Class,
    DependentClass,
    Export,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedSymbol {
    pub location: Location,
    pub name: String,
    pub kind: UnusedSymbolKind,
}

impl Diagnostic {
    pub fn render(&self, cwd: &std::path::Path) -> String {
        format!(
            "{}:{}:{} {} {} {}",
            display_path(&self.location.path, cwd).display(),
            self.location.line,
            self.location.column,
            self.severity,
            self.rule,
            self.message
        )
    }

    pub fn sort_key(&self) -> (String, usize, usize, &'static str) {
        (
            self.location.path.to_string_lossy().replace('\\', "/"),
            self.location.line,
            self.location.column,
            self.rule,
        )
    }
}

pub fn render_rich(diagnostics: &[Diagnostic], cwd: &std::path::Path, color: bool) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(diagnostics.len() + 3);
    for diagnostic in diagnostics {
        let severity = format!("[{}]", diagnostic.severity);
        let severity = match diagnostic.severity {
            Severity::Error => paint(&severity, "31", color),
            Severity::Warning => paint(&severity, "33", color),
            Severity::Off => severity,
        };
        let source = source_location(&diagnostic.location, cwd);
        let source = paint(&source, "34;4", color);
        lines.push(format!(
            "{severity} [{}] {source} - {}",
            diagnostic.rule,
            sentence(&diagnostic.message)
        ));
    }

    lines.push(String::new());
    lines.push(format!(
        "{} {} {}",
        paint("Errors", "31", color),
        paint("Warnings", "33", color),
        paint("Source", "34;4", color)
    ));
    let mut summaries: BTreeMap<PathBuf, (usize, usize, &Location)> = BTreeMap::new();
    for diagnostic in diagnostics {
        let summary = summaries
            .entry(diagnostic.location.path.clone())
            .or_insert((0, 0, &diagnostic.location));
        match diagnostic.severity {
            Severity::Error => summary.0 += 1,
            Severity::Warning => summary.1 += 1,
            Severity::Off => {}
        }
    }
    for (_, (errors, warnings, first_location)) in summaries {
        let errors = paint(&errors.to_string(), "31", color);
        let warnings = paint(&warnings.to_string(), "33", color);
        let source = paint(&source_location(first_location, cwd), "34;4", color);
        lines.push(format!("{errors} {warnings} {source}"));
    }
    lines.join("\n")
}

fn source_location(location: &Location, cwd: &std::path::Path) -> String {
    format!(
        "{}:{}:{}",
        display_path(&location.path, cwd).display(),
        location.line,
        location.column
    )
}

fn sentence(message: &str) -> String {
    let message = message.trim_end();
    if matches!(message.chars().last(), Some('.' | '!' | '?')) {
        message.to_owned()
    } else {
        format!("{message}.")
    }
}

fn paint(value: &str, ansi: &str, color: bool) -> String {
    if color {
        format!("\u{1b}[{ansi}m{value}\u{1b}[0m")
    } else {
        value.to_owned()
    }
}

pub fn location_at(path: &std::path::Path, source: &str, byte: usize) -> Location {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len() + 1, |(_, tail)| tail.len() + 1);
    Location {
        path: path.to_path_buf(),
        line,
        column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suppressions(source: &str) -> Result<Vec<Suppression>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        parse_suppressions(std::path::Path::new("example.ts"), source, tree.root_node())
    }

    fn diagnostic(path: &str, severity: Severity, rule: &'static str, line: usize) -> Diagnostic {
        Diagnostic {
            location: Location {
                path: PathBuf::from(path),
                line,
                column: 3,
            },
            severity,
            rule,
            message: "Example message".to_owned(),
        }
    }

    #[test]
    fn rich_output_has_diagnostics_and_per_source_totals() {
        let cwd = std::path::Path::new("project");
        let diagnostics = vec![
            diagnostic("project/src/a.ts", Severity::Error, "missing-symbol", 2),
            diagnostic(
                "project/src/a.ts",
                Severity::Warning,
                "dynamic-reference",
                8,
            ),
        ];
        let output = render_rich(&diagnostics, cwd, false);
        assert!(
            output.contains("[error] [missing-symbol] src/a.ts:2:3 - Example message."),
            "{output:?}"
        );
        assert!(output.contains("Errors Warnings Source\n1 1 src/a.ts:2:3"));
    }

    #[test]
    fn rich_output_colors_severity_and_source_only() {
        let diagnostic = diagnostic("src/a.ts", Severity::Error, "missing-symbol", 2);
        let output = render_rich(&[diagnostic], std::path::Path::new("."), true);
        assert!(output.contains("\u{1b}[31m[error]\u{1b}[0m"));
        assert!(output.contains(
            "\u{1b}[31mErrors\u{1b}[0m \u{1b}[33mWarnings\u{1b}[0m \u{1b}[34;4mSource\u{1b}[0m"
        ));
        assert!(
            output.contains("\u{1b}[34;4msrc/a.ts:2:3\u{1b}[0m"),
            "{output:?}"
        );
        assert!(output.contains(" - Example message."));
    }

    #[test]
    fn parses_single_line_all_rule_and_nested_suppressions() {
        let source = "// @sc-ignore dynamic-reference\none();\n// @sc-ignore\ntwo();\n/* @sc-ignore-start unused-class */\nthree();\n/* @sc-ignore-start naming-convention-local */\nfour();\n/* @sc-ignore-end */\nfive();\n/* @sc-ignore-end */\nsix();";
        let suppressions = suppressions(source).unwrap();
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(2, "dynamic-reference"))
        );
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(4, "missing-symbol"))
        );
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(6, "unused-class"))
        );
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(8, "unused-class"))
        );
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(8, "naming-convention-local"))
        );
        assert!(
            suppressions
                .iter()
                .any(|item| item.suppresses(10, "unused-class"))
        );
        assert!(
            !suppressions
                .iter()
                .any(|item| item.suppresses(12, "unused-class"))
        );
    }

    #[test]
    fn rejects_invalid_suppression_directives_with_locations() {
        for (source, expected) in [
            (
                "/* @sc-ignore imaginary-rule */\nconst a = 1;",
                "example.ts:1:1",
            ),
            ("/* @sc-ignore-end */", "unmatched @sc-ignore-end"),
            (
                "/* @sc-ignore-start */\nconst a = 1;",
                "unmatched @sc-ignore-start",
            ),
            (
                "/* @sc-ignore-end unused-class */",
                "does not accept rule names",
            ),
            ("/* @sc-ignore-forever */", "unknown suppression directive"),
        ] {
            let error = suppressions(source).unwrap_err().to_string();
            assert!(error.contains(expected), "{error:?}");
        }
    }

    #[test]
    fn ignores_directive_text_outside_comments() {
        let parsed = suppressions(r#"const value = "// @sc-ignore";"#).unwrap();
        assert!(parsed.is_empty());
    }
}
