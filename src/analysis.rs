use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use crate::{
    config::Config,
    diagnostic::{Diagnostic, Location, Severity, Suppression, UnusedSymbol, UnusedSymbolKind},
    stylesheet::{ClassScope, Declaration, Stylesheet, SymbolKind},
    typescript::TypeScriptModule,
};

pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
    pub unused_symbols: Vec<UnusedSymbol>,
}

pub fn analyze(
    config: &Config,
    stylesheets: Vec<Stylesheet>,
    modules: Vec<TypeScriptModule>,
) -> AnalysisResult {
    let mut diagnostics = Vec::new();
    let mut unused_symbols = Vec::new();
    let mut sheets: BTreeMap<PathBuf, Stylesheet> = stylesheets
        .into_iter()
        .map(|stylesheet| (stylesheet.path.clone(), stylesheet))
        .collect();
    let suppressions: BTreeMap<PathBuf, Vec<Suppression>> = sheets
        .values()
        .map(|stylesheet| (stylesheet.path.clone(), stylesheet.suppressions.clone()))
        .chain(
            modules
                .iter()
                .map(|module| (module.path.clone(), module.suppressions.clone())),
        )
        .collect();
    let mut used: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    let mut valid_dependent = BTreeSet::new();
    let mut suppress_unused = BTreeSet::new();

    for stylesheet in sheets.values() {
        validate_declarations(config, stylesheet, &mut diagnostics);
        for location in &stylesheet.empty_rules {
            emit(
                config,
                &mut diagnostics,
                "empty-rule",
                location.clone(),
                "Empty style rule",
            );
        }
        if is_module_stylesheet(&stylesheet.path) {
            for import in &stylesheet.imports {
                if imports_module(config, &stylesheet.path, &import.specifier, &sheets) {
                    emit(
                        config,
                        &mut diagnostics,
                        "module-to-module-import",
                        import.location.clone(),
                        format!(
                            "Avoid importing CSS Module '{}' into another CSS Module; its classes are merged into the importing module's public class map and can accumulate transitively",
                            import.specifier
                        ),
                    );
                }
            }
        }
        if !stylesheet.dynamic_locations.is_empty() {
            suppress_unused.insert(stylesheet.path.clone());
        }
        for location in &stylesheet.dynamic_locations {
            emit(
                config,
                &mut diagnostics,
                "dynamic-reference",
                location.clone(),
                "Cannot statically analyze an interpolated selector; prefer a literal class name",
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
                        "Stylesheet reference '{}' does not follow the selected convention",
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
        if config.severity("unused-dependent-class") != Severity::Off {
            for usage in &module.class_name_usages {
                let Some(stylesheet) = sheets.get(&usage.stylesheet) else {
                    continue;
                };
                let names: BTreeSet<_> = usage
                    .references
                    .iter()
                    .flat_map(|reference| config.convention.style_candidates(&reference.name))
                    .collect();
                for reference in &usage.references {
                    let expected: BTreeSet<_> = config
                        .convention
                        .style_candidates(&reference.name)
                        .into_iter()
                        .collect();
                    let declarations: Vec<_> = stylesheet
                        .dependent_declarations
                        .iter()
                        .filter(|declaration| {
                            expected.contains(&declaration.name)
                                && !stylesheet
                                    .suppresses(&declaration.location, "unused-dependent-class")
                        })
                        .collect();
                    if declarations.is_empty() {
                        continue;
                    }
                    let has_independent = expected
                        .iter()
                        .any(|name| has_independent_declaration(stylesheet, name));
                    let mut valid = false;
                    for declaration in declarations {
                        if declaration.prerequisite_paths.iter().any(|path| {
                            path.iter().all(|prerequisite| names.contains(prerequisite))
                        }) {
                            valid = true;
                            valid_dependent.insert((
                                usage.stylesheet.clone(),
                                declaration.location.line,
                                declaration.location.column,
                            ));
                        }
                    }
                    if !valid && !has_independent {
                        emit(
                            config,
                            &mut diagnostics,
                            "unused-dependent-class",
                            reference.location.clone(),
                            format!(
                                "Dependent class '{}' is used without a matching prerequisite class",
                                reference.name
                            ),
                        );
                    }
                }
            }
        }
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
                "Dynamic class reference cannot be verified; consider using an explicit mapper",
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
                        "Reference '{}' does not follow the selected TypeScript convention",
                        reference.name
                    ),
                );
            }
            let expected: BTreeSet<_> = config
                .convention
                .style_candidates(&reference.name)
                .into_iter()
                .collect();
            let found: BTreeSet<_> = stylesheet
                .declarations
                .iter()
                .filter(|declaration| {
                    expected.contains(&declaration.name) && declaration.scope == ClassScope::Local
                })
                .map(|declaration| declaration.name.clone())
                .collect();
            if !found.is_empty() {
                used.entry(reference.stylesheet).or_default().extend(found);
            } else {
                emit(
                    config,
                    &mut diagnostics,
                    "missing-symbol",
                    reference.location,
                    format!(
                        "Reference '{}' has no matching class or :export declaration in {}",
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
        let dependent_rule_enabled = config.severity("unused-dependent-class") != Severity::Off;
        let dependent_locations: BTreeSet<_> = stylesheet
            .dependent_declarations
            .iter()
            .filter(|declaration| {
                !stylesheet.suppresses(&declaration.location, "unused-dependent-class")
            })
            .map(|declaration| (declaration.location.line, declaration.location.column))
            .collect();
        let mut first_declaration: BTreeMap<(u8, String), &Declaration> = BTreeMap::new();
        for declaration in &stylesheet.declarations {
            if declaration.kind == SymbolKind::Class && declaration.scope == ClassScope::Global {
                continue;
            }
            let kind = match declaration.kind {
                SymbolKind::Class => 0,
                SymbolKind::Export => 1,
            };
            let rule = match declaration.kind {
                SymbolKind::Class => "unused-class",
                SymbolKind::Export => "unused-export",
            };
            let specialized = dependent_rule_enabled
                && declaration.kind == SymbolKind::Class
                && dependent_locations
                    .contains(&(declaration.location.line, declaration.location.column));
            if !specialized
                && !used_names.is_some_and(|names| names.contains(&declaration.name))
                && config.severity(rule) != Severity::Off
            {
                unused_symbols.push(UnusedSymbol {
                    location: declaration.location.clone(),
                    name: declaration.name.clone(),
                    kind: match declaration.kind {
                        SymbolKind::Class => UnusedSymbolKind::Class,
                        SymbolKind::Export => UnusedSymbolKind::Export,
                    },
                });
            }
            if !specialized {
                first_declaration
                    .entry((kind, declaration.name.clone()))
                    .or_insert(declaration);
            }
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
                format!("Unused {label} '{name}'"),
            );
        }
        if dependent_rule_enabled && !suppress_unused.contains(path) {
            for declaration in &stylesheet.dependent_declarations {
                if stylesheet.suppresses(&declaration.location, "unused-dependent-class") {
                    continue;
                }
                let key = (
                    path.clone(),
                    declaration.location.line,
                    declaration.location.column,
                );
                if valid_dependent.contains(&key) {
                    continue;
                }
                unused_symbols.push(UnusedSymbol {
                    location: declaration.location.clone(),
                    name: declaration.name.clone(),
                    kind: UnusedSymbolKind::DependentClass,
                });
                emit(
                    config,
                    &mut diagnostics,
                    "unused-dependent-class",
                    declaration.location.clone(),
                    format!(
                        "Dependent class '{}' has no valid className usage",
                        declaration.name
                    ),
                );
            }
        }
    }
    diagnostics
        .retain(|diagnostic| !is_suppressed(&suppressions, &diagnostic.location, diagnostic.rule));
    unused_symbols.retain(|symbol| {
        let rule = match symbol.kind {
            UnusedSymbolKind::Class => "unused-class",
            UnusedSymbolKind::DependentClass => "unused-dependent-class",
            UnusedSymbolKind::Export => "unused-export",
        };
        !is_suppressed(&suppressions, &symbol.location, rule)
    });
    AnalysisResult {
        diagnostics,
        unused_symbols,
    }
}

fn is_suppressed(
    suppressions: &BTreeMap<PathBuf, Vec<Suppression>>,
    location: &Location,
    rule: &str,
) -> bool {
    suppressions.get(&location.path).is_some_and(|items| {
        items
            .iter()
            .any(|item| item.suppresses(location.line, rule))
    })
}

fn has_independent_declaration(stylesheet: &Stylesheet, name: &str) -> bool {
    stylesheet.declarations.iter().any(|declaration| {
        declaration.kind == SymbolKind::Class
            && declaration.scope == ClassScope::Local
            && declaration.name == name
            && !stylesheet.is_dependent_declaration(declaration)
    })
}

fn is_module_stylesheet(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".module.css") || name.ends_with(".module.scss"))
}

fn imports_module(
    config: &Config,
    importer: &std::path::Path,
    specifier: &str,
    sheets: &BTreeMap<PathBuf, Stylesheet>,
) -> bool {
    if specifier.starts_with("sass:") || specifier.contains("://") {
        return false;
    }
    let explicit_module = specifier.ends_with(".module.css")
        || specifier.ends_with(".module.scss")
        || specifier.ends_with(".module");
    let value = std::path::Path::new(specifier);
    let mut bases = Vec::new();
    if value.is_absolute() {
        bases.push(value.to_path_buf());
    } else {
        if let Some(parent) = importer.parent() {
            bases.push(parent.join(value));
        }
        bases.push(config.cwd.join(value));
        bases.push(config.source.join(value));
    }
    for base in bases {
        for candidate in sass_candidates(&base) {
            let candidate = std::fs::canonicalize(&candidate).unwrap_or(candidate);
            if sheets.contains_key(&candidate) && is_module_stylesheet(&candidate) {
                return true;
            }
        }
    }
    explicit_module
}

fn sass_candidates(base: &std::path::Path) -> Vec<PathBuf> {
    let mut candidates = vec![base.to_path_buf()];
    if base.extension().is_none() {
        candidates.push(base.with_extension("scss"));
        candidates.push(base.with_extension("css"));
    }
    if let (Some(parent), Some(name)) = (base.parent(), base.file_name()) {
        let partial = parent.join(format!("_{}", name.to_string_lossy()));
        if base.extension().is_none() {
            candidates.push(partial.with_extension("scss"));
            candidates.push(partial.with_extension("css"));
        } else {
            candidates.push(partial);
        }
    }
    candidates.extend([
        base.join("_index.scss"),
        base.join("index.scss"),
        base.join("_index.css"),
        base.join("index.css"),
    ]);
    candidates
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
                    "Declaration '{}' does not follow the selected stylesheet convention",
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
                        "Ambiguous symbol '{}' is declared as both a class and :export (first at {}:{})",
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
