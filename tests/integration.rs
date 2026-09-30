use std::fs;

use style_contract::{cli::Cli, config::Config, naming::Convention};

fn cli(source: std::path::PathBuf, convention: Convention) -> Cli {
    Cli {
        convention,
        source,
        exclude: vec![],
        include: vec![],
        ignore_exports: false,
        rule: vec![],
        tsconfig: "tsconfig.json".into(),
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
    assert!(rules.contains(&"no-missing-symbols"));
    assert!(rules.contains(&"no-unused-classes"));
    assert!(rules.contains(&"no-unused-exports"));
    assert!(result.has_errors);
}

#[test]
fn dynamic_reference_warns_and_suppresses_unused_for_only_its_stylesheet() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("dynamic.css"), ".one {}\n.two {}").unwrap();
    fs::write(src.join("static.css"), ".unused {}").unwrap();
    fs::write(
        src.join("index.ts"),
        r#"import dynamic from "./dynamic.css";
import statik from "./static.css";
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
            .filter(|item| item.rule == "no-dynamic-references")
            .count(),
        1
    );
    let unused: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| item.rule == "no-unused-classes")
        .collect();
    assert_eq!(unused.len(), 1);
    assert!(unused[0].location.path.ends_with("static.css"));
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
    fs::write(src.join("unused.css"), ".unused {}").unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["no-unused-classes:warning".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(!result.has_errors);
    assert_eq!(result.diagnostics[0].severity.to_string(), "warning");
}
