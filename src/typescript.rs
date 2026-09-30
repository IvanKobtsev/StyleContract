use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use rayon::prelude::*;
use regex::Regex;
use tree_sitter::{Node, Parser};

use crate::{
    diagnostic::{Location, location_at},
    resolver::Resolver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSyntax {
    Dot,
    Bracket,
    Destructure,
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub name: String,
    pub stylesheet: PathBuf,
    pub location: Location,
    pub syntax: AccessSyntax,
}

#[derive(Debug, Clone)]
pub struct DynamicReference {
    pub stylesheet: PathBuf,
    pub location: Location,
}

#[derive(Debug)]
pub struct TypeScriptModule {
    pub references: Vec<Reference>,
    pub dynamic: Vec<DynamicReference>,
}

pub fn parse_all(paths: &[PathBuf], resolver: &Resolver) -> Result<Vec<TypeScriptModule>> {
    paths.par_iter().map(|path| parse(path, resolver)).collect()
}

pub fn parse(path: &std::path::Path, resolver: &Resolver) -> Result<TypeScriptModule> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("could not read TypeScript file {}", path.display()))?;
    let mut parser = Parser::new();
    let language = if path.extension().and_then(|value| value.to_str()) == Some("tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    parser
        .set_language(&language)
        .map_err(|error| anyhow!("could not load TypeScript parser: {error}"))?;
    let tree = parser.parse(&source, None).ok_or_else(|| {
        anyhow!(
            "TypeScript parser did not return a tree for {}",
            path.display()
        )
    })?;
    if tree.root_node().has_error() {
        return Err(anyhow!(
            "could not parse TypeScript file {}",
            path.display()
        ));
    }

    let imports = collect_imports(tree.root_node(), source.as_bytes(), path, resolver)?;
    let mut module = TypeScriptModule {
        references: vec![],
        dynamic: vec![],
    };
    walk_accesses(
        tree.root_node(),
        source.as_bytes(),
        path,
        &imports,
        &mut module,
    );
    collect_destructuring(&source, path, &imports, &mut module);
    Ok(module)
}

fn collect_imports(
    root: Node<'_>,
    source: &[u8],
    path: &std::path::Path,
    resolver: &Resolver,
) -> Result<BTreeMap<String, PathBuf>> {
    let import_re = Regex::new(
        r#"^\s*import\s+(?:([A-Za-z_$][A-Za-z0-9_$]*)|\*\s+as\s+([A-Za-z_$][A-Za-z0-9_$]*))\s+from\s+['\"]([^'\"]+\.(?:css|scss))['\"]"#,
    )
    .expect("valid regex");
    let mut imports = BTreeMap::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() != "import_statement" {
            continue;
        }
        let text = child
            .utf8_text(source)
            .expect("tree-sitter ranges are UTF-8");
        let Some(captures) = import_re.captures(text) else {
            continue;
        };
        let alias = captures
            .get(1)
            .or_else(|| captures.get(2))
            .expect("one import form matched")
            .as_str();
        let specifier = captures.get(3).expect("specifier matched").as_str();
        let stylesheet = resolver.resolve(path, specifier)?;
        imports.insert(alias.to_owned(), stylesheet);
    }
    Ok(imports)
}

fn walk_accesses(
    node: Node<'_>,
    source: &[u8],
    path: &std::path::Path,
    imports: &BTreeMap<String, PathBuf>,
    module: &mut TypeScriptModule,
) {
    match node.kind() {
        "member_expression" => {
            if let (Some(object), Some(property)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("property"),
            ) {
                let object_text = object.utf8_text(source).unwrap_or_default();
                if let Some(stylesheet) = imports.get(object_text) {
                    let name = property.utf8_text(source).unwrap_or_default();
                    module.references.push(Reference {
                        name: name.to_owned(),
                        stylesheet: stylesheet.clone(),
                        location: location_at(
                            path,
                            std::str::from_utf8(source).unwrap(),
                            property.start_byte(),
                        ),
                        syntax: AccessSyntax::Dot,
                    });
                }
            }
        }
        "subscript_expression" => {
            if let (Some(object), Some(index)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("index"),
            ) {
                let object_text = object.utf8_text(source).unwrap_or_default();
                if let Some(stylesheet) = imports.get(object_text) {
                    let index_text = index.utf8_text(source).unwrap_or_default();
                    if let Some(name) = string_literal(index_text) {
                        module.references.push(Reference {
                            name: name.to_owned(),
                            stylesheet: stylesheet.clone(),
                            location: location_at(
                                path,
                                std::str::from_utf8(source).unwrap(),
                                index.start_byte() + 1,
                            ),
                            syntax: AccessSyntax::Bracket,
                        });
                    } else {
                        module.dynamic.push(DynamicReference {
                            stylesheet: stylesheet.clone(),
                            location: location_at(
                                path,
                                std::str::from_utf8(source).unwrap(),
                                index.start_byte(),
                            ),
                        });
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_accesses(child, source, path, imports, module);
    }
}

fn collect_destructuring(
    source: &str,
    path: &std::path::Path,
    imports: &BTreeMap<String, PathBuf>,
    module: &mut TypeScriptModule,
) {
    let item_re = Regex::new(
        r#"(?:['\"]([^'\"]+)['\"]|([A-Za-z_$][A-Za-z0-9_$]*))(?:\s*:\s*[A-Za-z_$][A-Za-z0-9_$]*)?"#,
    )
    .expect("valid regex");
    for (alias, stylesheet) in imports {
        let pattern = format!(
            r"(?:const|let|var)\s*\{{([^}}]+)\}}\s*=\s*{}\b",
            regex::escape(alias)
        );
        let destructure_re = Regex::new(&pattern).expect("escaped alias makes valid regex");
        for capture in destructure_re.captures_iter(source) {
            let body = capture.get(1).expect("body matched");
            for item in item_re.captures_iter(body.as_str()) {
                let matched = item.get(1).or_else(|| item.get(2)).expect("name matched");
                module.references.push(Reference {
                    name: matched.as_str().to_owned(),
                    stylesheet: stylesheet.clone(),
                    location: location_at(path, source, body.start() + matched.start()),
                    syntax: AccessSyntax::Destructure,
                });
            }
        }
    }
}

fn string_literal(value: &str) -> Option<&str> {
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'\'' || first == b'"') && first == last {
            return Some(&value[1..value.len() - 1]);
        }
    }
    None
}
