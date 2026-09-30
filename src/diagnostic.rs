use std::{fmt, path::PathBuf};

use crate::display_path;

pub const RULES: [&str; 5] = [
    "no-missing-symbols",
    "no-unused-classes",
    "no-unused-exports",
    "naming-convention",
    "no-dynamic-references",
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
