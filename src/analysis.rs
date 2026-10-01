use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::{
    config::Config,
    diagnostic::{Diagnostic, Location, Severity},
    stylesheet::{ClassScope, Declaration, Stylesheet, SymbolKind},
    typescript::TypeScriptModule,
};

pub fn analyze(
    config: &Config,
    stylesheets: Vec<Stylesheet>,
    modules: Vec<TypeScriptModule>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut sheets: BTreeMap<PathBuf, Stylesheet> = stylesheets
        .into_iter()
        .map(|stylesheet| (stylesheet.path.clone(), stylesheet))
        .collect();
    let mut used: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    let mut suppress_unused = BTreeSet::new();

    for stylesheet in sheets.values() {
        validate_declarations(config, stylesheet, &mut diagnostics);
        if !stylesheet.dynamic_locations.is_empty() {
            suppress_unused.insert(stylesheet.path.clone());
        }
        for location in &stylesheet.dynamic_locations {
            emit(
                config,
                &mut diagnostics,
                "dynamic-reference",
                location.clone(),
                "cannot statically analyze an interpolated selector; prefer a literal class name",
            );
        }
    }

    for stylesheet in sheets.values() {
        for reference in &stylesheet.references {
            let convention_rule = if reference.scope == ClassScope::Global {
                "naming-convention-global"
            } else {
                "naming-convention-local"
            };
            if !config.convention.valid_style_name(&reference.name) {
                emit(
                    config,
                    &mut diagnostics,
                    convention_rule,
                    reference.location.clone(),
                    format!(
                        "stylesheet reference '{}' does not follow the selected convention",
                        reference.name
                    ),
                );
            }
            let Some(target) = sheets.get(&reference.stylesheet) else {
                continue;
            };
            if target.declarations.iter().any(|declaration| {
                declaration.kind == SymbolKind::Class
                    && declaration.scope == ClassScope::Local
                    && declaration.name == reference.name
            }) {
                used.entry(reference.stylesheet.clone())
                    .or_default()
                    .insert(reference.name.clone());
            }
        }
    }

    for module in modules {
        for dynamic in module.dynamic {
            if !sheets.contains_key(&dynamic.stylesheet) {
                continue;
            }
            suppress_unused.insert(dynamic.stylesheet);
            emit(
                config,
                &mut diagnostics,
                "dynamic-reference",
                dynamic.location,
                "dynamic class reference cannot be verified; consider using an explicit mapper",
            );
        }
        for reference in module.references {
            let Some(stylesheet) = sheets.get(&reference.stylesheet) else {
                continue;
            };
            if !config.convention.valid_code_name(&reference.name) {
                emit(
                    config,
                    &mut diagnostics,
                    "naming-convention-local",
                    reference.location.clone(),
                    format!(
                        "reference '{}' does not follow the selected TypeScript convention",
                        reference.name
                    ),
                );
            }
            let expected = config.convention.code_to_style(&reference.name);
            let found = stylesheet.declarations.iter().any(|declaration| {
                declaration.name == expected && declaration.scope == ClassScope::Local
            });
            if found {
                used.entry(reference.stylesheet)
                    .or_default()
                    .insert(expected);
            } else {
                emit(
                    config,
                    &mut diagnostics,
                    "missing-symbol",
                    reference.location,
                    format!(
                        "'{}' has no matching class or :export declaration in {}",
                        reference.name,
                        stylesheet
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    ),
                );
            }
        }
    }

    for (path, stylesheet) in &mut sheets {
        if suppress_unused.contains(path) {
            continue;
        }
        let used_names = used.get(path);
        let mut first_declaration: BTreeMap<(u8, String), &Declaration> = BTreeMap::new();
        for declaration in &stylesheet.declarations {
            if declaration.kind == SymbolKind::Class && declaration.scope == ClassScope::Global {
                continue;
            }
            let kind = match declaration.kind {
                SymbolKind::Class => 0,
                SymbolKind::Export => 1,
            };
            first_declaration
                .entry((kind, declaration.name.clone()))
                .or_insert(declaration);
        }
        for ((_, name), declaration) in first_declaration {
            if used_names.is_some_and(|names| names.contains(&name)) {
                continue;
            }
            let (rule, label) = match declaration.kind {
                SymbolKind::Class => ("unused-class", "class"),
                SymbolKind::Export => ("unused-export", ":export key"),
            };
            emit(
                config,
                &mut diagnostics,
                rule,
                declaration.location.clone(),
                format!("unused {label} '{name}'"),
            );
        }
    }
    diagnostics
}

fn validate_declarations(
    config: &Config,
    stylesheet: &Stylesheet,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut names: BTreeMap<&str, (&Declaration, SymbolKind)> = BTreeMap::new();
    for declaration in &stylesheet.declarations {
        let convention_rule = if declaration.scope == ClassScope::Global {
            "naming-convention-global"
        } else {
            "naming-convention-local"
        };
        if !config.convention.valid_style_name(&declaration.name) {
            emit(
                config,
                diagnostics,
                convention_rule,
                declaration.location.clone(),
                format!(
                    "declaration '{}' does not follow the selected stylesheet convention",
                    declaration.name
                ),
            );
        }
        if declaration.scope == ClassScope::Global {
            continue;
        }
        if let Some((previous, previous_kind)) = names.get(declaration.name.as_str()) {
            if *previous_kind != declaration.kind {
                emit(
                    config,
                    diagnostics,
                    "naming-convention-local",
                    declaration.location.clone(),
                    format!(
                        "ambiguous symbol '{}' is declared as both a class and :export (first at {}:{})",
                        declaration.name, previous.location.line, previous.location.column
                    ),
                );
            }
        } else {
            names.insert(&declaration.name, (declaration, declaration.kind));
        }
    }
}

fn emit(
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
    rule: &'static str,
    location: Location,
    message: impl Into<String>,
) {
    let severity = config.severity(rule);
    if severity == Severity::Off {
        return;
    }
    diagnostics.push(Diagnostic {
        location,
        severity,
        rule,
        message: message.into(),
    });
}
