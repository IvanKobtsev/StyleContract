use std::fs;

use style_contract::{
    cli::{Cli, OutputStyle},
    config::Config,
    naming::Convention,
};

fn cli(source: std::path::PathBuf, convention: Convention) -> Cli {
    Cli {
        convention: Some(convention),
        config: None,
        output: Some(OutputStyle::Rich),
        source: Some(source),
        exclude: vec![],
        include: vec![],
        ignore_exports: Some(false),
        rule: vec![],
        tsconfig: Some("tsconfig.json".into()),
    }
}

#[test]
fn reports_missing_and_unused_symbols_across_scss_and_tsx() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".primary-button { color: red; }\n.unused-class {}\n:export { brand-color: red; }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
const ok = styles.primaryButton;
const missing = styles.missingName;
export const Button = () => <div className={ok + missing} />;
"#,
    )
    .unwrap();

    let config = Config::from_cli(
        cli(src, Convention::CamelToKebab),
        temp.path().to_path_buf(),
    )
    .unwrap();
    let result = style_contract::run(&config).unwrap();
    let rules: Vec<_> = result.diagnostics.iter().map(|item| item.rule).collect();
    assert!(rules.contains(&"missing-symbol"));
    assert!(rules.contains(&"unused-class"));
    assert!(rules.contains(&"unused-export"));
    assert!(result.has_errors);
}

#[test]
fn dynamic_reference_warns_and_suppresses_unused_for_only_its_stylesheet() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("dynamic.module.css"), ".one {}\n.two {}").unwrap();
    fs::write(src.join("static.module.css"), ".unused {}").unwrap();
    fs::write(
        src.join("index.ts"),
        r#"import dynamic from "./dynamic.module.css";
import statik from "./static.module.css";
declare const name: string;
console.log(dynamic[name], statik);
"#,
    )
    .unwrap();

    let config =
        Config::from_cli(cli(src, Convention::CamelCase), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|item| item.rule == "dynamic-reference")
            .count(),
        1
    );
    let unused: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| item.rule == "unused-class")
        .collect();
    assert_eq!(unused.len(), 1);
    assert!(unused[0].location.path.ends_with("static.module.css"));
}

#[test]
fn resolves_tsconfig_path_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    let styles = src.join("styles");
    fs::create_dir_all(&styles).unwrap();
    fs::write(styles.join("card.module.css"), ".card-root {}").unwrap();
    fs::write(
        src.join("card.ts"),
        "import styles from '@styles/card.module.css';\nconsole.log(styles.cardRoot);",
    )
    .unwrap();
    fs::write(
        temp.path().join("tsconfig.json"),
        r#"{
          // JSONC is accepted
          "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@styles/*": ["src/styles/*"], },
          },
        }"#,
    )
    .unwrap();

    let config = Config::from_cli(
        cli(src, Convention::CamelToKebab),
        temp.path().to_path_buf(),
    )
    .unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rule_overrides_change_exit_relevant_severity() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("unused.module.css"), ".unused {}").unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-class:warning".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(!result.has_errors);
    assert_eq!(result.diagnostics[0].severity.to_string(), "warning");
}

#[test]
fn global_classes_only_receive_global_convention_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("component.module.scss"),
        ".localClass {} :global(.MuiDialog-paper) {}",
    )
    .unwrap();
    fs::write(src.join("global.scss"), ".External-widget {}").unwrap();
    fs::write(
        src.join("component.ts"),
        r#"import styles from "./component.module.scss";
console.log(styles.localClass, styles.MuiDialogPaper);
"#,
    )
    .unwrap();
    let config =
        Config::from_cli(cli(src, Convention::CamelCase), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == "naming-convention-global")
            .count(),
        2
    );
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == "unused-class")
            .count(),
        0
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule == "missing-symbol" && diagnostic.message.contains("MuiDialogPaper")
    }));
}

#[test]
fn stylesheet_references_count_as_class_usage() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("base.module.css"), ".baseClass {}").unwrap();
    fs::write(
        src.join("component.module.scss"),
        r#".extendedClass { @extend .localBase; }
.localBase {}
.composedClass { composes: baseClass from "./base.module.css"; }
"#,
    )
    .unwrap();
    fs::write(
        src.join("component.ts"),
        r#"import styles from "./component.module.scss";
console.log(styles.extendedClass, styles.composedClass);
"#,
    )
    .unwrap();
    let config =
        Config::from_cli(cli(src, Convention::CamelCase), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "unused-class"),
        "{:#?}",
        result.diagnostics
    );
}
