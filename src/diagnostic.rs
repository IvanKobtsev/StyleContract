use std::{collections::BTreeMap, fmt, path::PathBuf};

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
}
