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
        ".primary-button { color: red; }\n.unused-class { color: inherit; }\n:export { brand-color: red; }",
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
    fs::write(
        src.join("dynamic.module.css"),
        ".one { color: inherit; }\n.two { color: inherit; }",
    )
    .unwrap();
    fs::write(src.join("static.module.css"), ".unused { color: inherit; }").unwrap();
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
    fs::write(
        styles.join("card.module.css"),
        ".card-root { color: inherit; }",
    )
    .unwrap();
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
    fs::write(src.join("unused.module.css"), ".unused { color: inherit; }").unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-class:warning".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(!result.has_errors);
    assert_eq!(result.diagnostics[0].severity.to_string(), "warning");
}

#[test]
fn empty_rule_defaults_to_warning_and_respects_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("empty.module.css"), ".empty {}").unwrap();

    let mut warning_args = cli(src.clone(), Convention::CamelCase);
    warning_args.rule = vec!["unused-class:off".into()];
    let warning_config = Config::from_cli(warning_args, temp.path().to_path_buf()).unwrap();
    let warning = style_contract::run(&warning_config).unwrap();
    assert_eq!(warning.diagnostics.len(), 1);
    assert_eq!(warning.diagnostics[0].rule, "empty-rule");
    assert_eq!(warning.diagnostics[0].severity.to_string(), "warning");
    assert!(!warning.has_errors);

    let mut error_args = cli(src.clone(), Convention::CamelCase);
    error_args.rule = vec!["unused-class:off".into(), "empty-rule:error".into()];
    let error_config = Config::from_cli(error_args, temp.path().to_path_buf()).unwrap();
    assert!(style_contract::run(&error_config).unwrap().has_errors);

    let mut off_args = cli(src, Convention::CamelCase);
    off_args.rule = vec!["unused-class:off".into(), "empty-rule:off".into()];
    let off_config = Config::from_cli(off_args, temp.path().to_path_buf()).unwrap();
    assert!(
        style_contract::run(&off_config)
            .unwrap()
            .diagnostics
            .is_empty()
    );
}

#[test]
fn warns_for_module_to_module_sass_dependencies_only() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("styles")).unwrap();
    fs::write(
        src.join("styles/table.module.scss"),
        ".table { color: inherit; }",
    )
    .unwrap();
    fs::write(src.join("styles/variables.scss"), "$gap: 4px;").unwrap();
    fs::write(
        src.join("feature.module.scss"),
        "@use \"src/styles/table.module.scss\" as *;\n@use \"./styles/variables\" as *;\n.feature { padding: $gap; }",
    )
    .unwrap();

    let mut args = cli(src.clone(), Convention::CamelCase);
    args.rule = vec!["unused-class:off".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| item.rule == "module-to-module-import")
        .collect();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity.to_string(), "warning");
    assert!(diagnostics[0].message.contains("merged"));
    assert!(!result.has_errors);

    let mut off_args = cli(src, Convention::CamelCase);
    off_args.rule = vec![
        "unused-class:off".into(),
        "module-to-module-import:off".into(),
    ];
    let off_config = Config::from_cli(off_args, temp.path().to_path_buf()).unwrap();
    assert!(
        style_contract::run(&off_config)
            .unwrap()
            .diagnostics
            .iter()
            .all(|item| item.rule != "module-to-module-import")
    );
}

#[test]
fn global_classes_only_receive_global_convention_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("component.module.scss"),
        ".localClass { color: inherit; } :global(.MuiDialog-paper) { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("global.scss"),
        ".External-widget { color: inherit; }",
    )
    .unwrap();
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
    fs::write(
        src.join("base.module.css"),
        ".baseClass { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("component.module.scss"),
        r#".extendedClass { @extend .localBase; }
.localBase { color: inherit; }
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

#[test]
fn dependent_classes_require_a_prerequisite_in_the_same_class_name() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button { &.selected { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
export const Good = () => <button className={clsx(styles.button, true && styles.selected)} />;
export const Bad = () => <button className={styles.selected} />;
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let dependent: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "unused-dependent-class")
        .collect();
    assert_eq!(dependent.len(), 1, "{:#?}", result.diagnostics);
    assert!(dependent[0].location.path.ends_with("button.tsx"));
}

#[test]
fn dependent_classes_accept_clsx_usage_outside_class_name() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button { &.selected { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("button.ts"),
        r#"import clsx from "clsx";
import styles from "./button.module.scss";
export const selectedButton = clsx(styles.button, styles.selected);
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "unused-dependent-class"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn dependent_declaration_is_reported_without_any_valid_usage_and_not_duplicated() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button { &.selected { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
export const Bad = () => <button className={styles.selected} />;
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == "unused-dependent-class")
            .count(),
        2,
        "{:#?}",
        result.diagnostics
    );
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.rule != "unused-class" || !diagnostic.message.contains("selected")
    }));
}

#[test]
fn dependent_declaration_is_credited_when_the_class_is_also_standalone() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("scroll-controls.module.scss"),
        ".offset { color: blue; }\n.controls { &.offset { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("scroll-controls.tsx"),
        r#"import styles from "./scroll-controls.module.scss";
export const ScrollControls = () => (
    <div className={`${styles.controls} ${styles.offset}`} />
);
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "unused-dependent-class"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn combinators_and_has_selectors_are_not_dependent_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("beautiful-mentions-menu.module.scss"),
        ".menuItem:has(+ .menuItem:hover) { color: red; }\n.menu > .menuItem.active { color: blue; }",
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec![
        "unused-class:off".into(),
        "unused-dependent-class:error".into(),
    ];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "unused-dependent-class"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn next_line_directives_suppress_only_the_named_rule() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("ignored.module.scss"),
        "/* @sc-ignore unused-class, unused-dependent-class */\n.comment.highlighted { color: yellow; }\n/* @sc-ignore unused-class */\n.bad_name { color: red; }",
    )
    .unwrap();
    fs::write(src.join("comment-view.module.scss"), ".comment {}\n").unwrap();
    fs::write(
        src.join("use-highlighting.ts"),
        r#"import styles from "./comment-view.module.scss";
declare const name: string;
// @sc-ignore dynamic-reference
export const useHighlighting = () => styles[name];
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result.diagnostics.iter().all(|diagnostic| {
            !matches!(
                diagnostic.rule,
                "unused-dependent-class" | "unused-class" | "dynamic-reference"
            )
        }),
        "{:#?}",
        result.diagnostics
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "naming-convention-local"),
        "the directive must not suppress a different rule: {:#?}",
        result.diagnostics
    );
    assert!(result.unused_symbols.is_empty());
}

#[test]
fn dependent_rule_is_off_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button { &.selected { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
export const Bad = () => <button className={styles.selected} />;
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
            .all(|diagnostic| diagnostic.rule != "unused-dependent-class")
    );
}

#[test]
fn dependent_classes_accept_complete_alternative_paths_in_templates() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button, .link { &.selected { &.busy { color: red; } } }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
export const Button = () => <button className={`${styles.link} ${styles.selected} ${styles.busy}`} />;
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "unused-dependent-class"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn computed_access_suppresses_only_the_declaration_side_finding() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button { &.selected { color: red; } }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import styles from "./button.module.scss";
declare const name: string;
console.log(styles[name]);
export const Bad = () => <button className={styles.selected} />;
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::CamelCase);
    args.rule = vec!["unused-dependent-class:error".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let dependent: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "unused-dependent-class")
        .collect();
    assert_eq!(dependent.len(), 1, "{:#?}", result.diagnostics);
    assert!(dependent[0].location.path.ends_with("button.tsx"));
}
