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
        print_only_errors: None,
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
fn no_convention_matches_camel_references_to_both_style_spellings() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("mixed.module.scss"),
        ".myClass { color: red; }\n.my-class { color: blue; }\n.snake_case { color: green; }",
    )
    .unwrap();
    fs::write(
        src.join("mixed.ts"),
        r#"import styles from "./mixed.module.scss";
console.log(styles.myClass, styles.snake_case);
"#,
    )
    .unwrap();
    let config = Config::from_cli(cli(src, Convention::None), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    assert!(
        result.diagnostics.iter().all(|diagnostic| {
            !matches!(
                diagnostic.rule,
                "missing-symbol"
                    | "unused-class"
                    | "naming-convention-local"
                    | "naming-convention-global"
            )
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn no_convention_keeps_kebab_references_exact() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("mixed.module.scss"),
        ".myClass { color: red; }\n.my-class { color: blue; }",
    )
    .unwrap();
    fs::write(
        src.join("mixed.ts"),
        r#"import styles from "./mixed.module.scss";
console.log(styles["my-class"]);
"#,
    )
    .unwrap();
    let config = Config::from_cli(cli(src, Convention::None), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let unused: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "unused-class")
        .collect();
    assert_eq!(unused.len(), 1, "{:#?}", result.diagnostics);
    assert!(unused[0].message.contains("myClass"));
}

#[test]
fn no_convention_expands_dependent_names_in_clsx_groups() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("button.module.scss"),
        ".button.myState { color: red; }\n.button.my-state { color: blue; }\n.button.jsxState { color: green; }\n.button.jsx-state { color: purple; }",
    )
    .unwrap();
    fs::write(
        src.join("button.tsx"),
        r#"import clsx from "clsx";
import styles from "./button.module.scss";
export const classes = clsx(styles.button, styles.myState);
export const Button = () => <button className={`${styles.button} ${styles.jsxState}`} />;
"#,
    )
    .unwrap();
    let mut args = cli(src, Convention::None);
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
    warning_args.print_only_errors = Some(true);
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
fn class_leak_warns_only_for_direct_imports_that_declare_classes() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("styles")).unwrap();
    fs::write(
        src.join("styles/table.module.scss"),
        ".table { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("styles/global.scss"),
        ".global { color: inherit; }",
    )
    .unwrap();
    fs::write(src.join("styles/variables.scss"), "$gap: 4px;").unwrap();
    fs::write(src.join("styles/empty.module.scss"), "$gap: 4px;").unwrap();
    fs::write(
        src.join("styles/transitive.scss"),
        "@use './global.scss';\n$gap: 4px;",
    )
    .unwrap();
    fs::write(
        src.join("feature.module.scss"),
        "@use \"src/styles/table.module.scss\" as *;\n@use \"./styles/global\";\n@use \"./styles/variables\" as *;\n@use \"./styles/empty.module\";\n@use \"./styles/transitive\";\n.feature { padding: $gap; }",
    )
    .unwrap();

    let mut args = cli(src.clone(), Convention::CamelCase);
    args.rule = vec!["unused-class:off".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| item.rule == "class-leak")
        .collect();
    assert_eq!(diagnostics.len(), 2, "{:#?}", result.diagnostics);
    assert_eq!(diagnostics[0].severity.to_string(), "warning");
    assert!(
        diagnostics[0]
            .message
            .contains("leaks its class declarations")
    );
    assert!(diagnostics[0].message.contains("TypeScript"));
    assert!(!result.has_errors);

    let mut off_args = cli(src, Convention::CamelCase);
    off_args.rule = vec!["unused-class:off".into(), "class-leak:off".into()];
    let off_config = Config::from_cli(off_args, temp.path().to_path_buf()).unwrap();
    assert!(
        style_contract::run(&off_config)
            .unwrap()
            .diagnostics
            .iter()
            .all(|item| item.rule != "class-leak")
    );
}

#[test]
fn class_leak_supports_targeted_suppression_and_rejects_the_old_rule_name() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("shared.scss"), ".shared { color: inherit; }").unwrap();
    fs::write(
        src.join("feature.module.scss"),
        "// @sc-ignore class-leak\n@use './shared';\n.feature { color: inherit; }",
    )
    .unwrap();
    let mut args = cli(src.clone(), Convention::None);
    args.rule = vec!["unused-class:off".into()];
    let config = Config::from_cli(args, temp.path().to_path_buf()).unwrap();
    assert!(
        style_contract::run(&config)
            .unwrap()
            .diagnostics
            .iter()
            .all(|item| item.rule != "class-leak")
    );

    let mut legacy = cli(src, Convention::None);
    legacy.rule = vec!["module-to-module-import:off".into()];
    assert!(
        Config::from_cli(legacy, temp.path().to_path_buf())
            .unwrap_err()
            .to_string()
            .contains("unknown rule 'module-to-module-import'")
    );
}

#[test]
fn leaked_class_usage_replaces_missing_symbol_and_credits_direct_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("shared.module.scss"),
        ".fromModule { color: inherit; }",
    )
    .unwrap();
    fs::write(src.join("global.scss"), ".fromGlobal { color: inherit; }").unwrap();
    fs::write(
        src.join("transitive.scss"),
        ".fromTransitive { color: inherit; }",
    )
    .unwrap();
    fs::write(src.join("bridge.scss"), "@use './transitive';\n$gap: 4px;").unwrap();
    fs::write(
        src.join("feature.module.scss"),
        "@use './shared.module';\n@use './global';\n@use './bridge';\n.local { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("feature.ts"),
        r#"import styles from "./feature.module.scss";
console.log(styles.local, styles.fromModule, styles.fromGlobal, styles.fromTransitive, styles.unknown);
"#,
    )
    .unwrap();

    let config =
        Config::from_cli(cli(src, Convention::CamelCase), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let leaked: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|item| item.rule == "leaked-class-usage")
        .collect();
    assert_eq!(leaked.len(), 2, "{:#?}", result.diagnostics);
    assert!(
        leaked
            .iter()
            .all(|item| item.severity.to_string() == "warning")
    );
    assert!(
        leaked
            .iter()
            .all(|item| item.message.contains("TypeScript"))
    );
    assert!(result.diagnostics.iter().all(|item| {
        item.rule != "missing-symbol"
            || (!item.message.contains("fromModule") && !item.message.contains("fromGlobal"))
    }));
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|item| item.rule == "missing-symbol")
            .count(),
        2,
        "{:#?}",
        result.diagnostics
    );
    assert!(
        result
            .diagnostics
            .iter()
            .all(|item| { item.rule != "unused-class" || !item.message.contains("fromModule") })
    );
}

#[test]
fn leaked_usage_rule_controls_only_its_warning() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("shared.module.scss"),
        ".shared { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("feature.module.scss"),
        "@use './shared.module.scss';\n.local { color: inherit; }",
    )
    .unwrap();
    fs::write(
        src.join("feature.ts"),
        "import styles from './feature.module.scss';\nconsole.log(styles.local, styles.shared);",
    )
    .unwrap();

    let mut error_args = cli(src.clone(), Convention::CamelCase);
    error_args.rule = vec!["class-leak:off".into(), "leaked-class-usage:error".into()];
    let error_config = Config::from_cli(error_args, temp.path().to_path_buf()).unwrap();
    let error_result = style_contract::run(&error_config).unwrap();
    assert!(
        error_result.diagnostics.iter().any(|item| {
            item.rule == "leaked-class-usage" && item.severity.to_string() == "error"
        })
    );
    assert!(
        error_result
            .diagnostics
            .iter()
            .all(|item| item.rule != "class-leak" && item.rule != "missing-symbol")
    );

    let mut off_args = cli(src.clone(), Convention::CamelCase);
    off_args.rule = vec!["class-leak:off".into(), "leaked-class-usage:off".into()];
    let off_config = Config::from_cli(off_args, temp.path().to_path_buf()).unwrap();
    let off_result = style_contract::run(&off_config).unwrap();
    assert!(
        off_result
            .diagnostics
            .iter()
            .all(|item| item.rule != "leaked-class-usage" && item.rule != "missing-symbol")
    );

    fs::write(
        src.join("feature.ts"),
        "import styles from './feature.module.scss';\n// @sc-ignore leaked-class-usage\nconsole.log(styles.shared);",
    )
    .unwrap();
    let mut suppressed_args = cli(src, Convention::CamelCase);
    suppressed_args.rule = vec!["class-leak:off".into()];
    let suppressed_config = Config::from_cli(suppressed_args, temp.path().to_path_buf()).unwrap();
    let suppressed_result = style_contract::run(&suppressed_config).unwrap();
    assert!(suppressed_result.diagnostics.iter().all(|item| {
        item.rule != "leaked-class-usage"
            && item.rule != "missing-symbol"
            && (item.rule != "unused-class" || !item.message.contains("shared"))
    }));
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
        "/* @sc-ignore unused-class, unused-dependent-class */\n.comment.highlighted { color: yellow; }\n/* @sc-ignore unused-class */\n.bad_name { color: red; }\n// @sc-ignore unused-class\n.scssOnly { color: blue; }",
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
fn indented_scss_directives_do_not_shift_to_adjacent_classes() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("steps.module.scss"),
        r#".step {
  // @sc-ignore
  &.failed { color: red; }

  // @sc-ignore

  &.passed { color: green; }
}

.blockedStep {
  // @sc-ignore-start
  &.blocked { color: red; }
  // @sc-ignore-end
  &.ready { color: green; }
}
"#,
    )
    .unwrap();
    fs::write(
        src.join("steps.ts"),
        "import styles from './steps.module.scss';\nconsole.log(styles.step, styles.blockedStep);",
    )
    .unwrap();

    let config =
        Config::from_cli(cli(src, Convention::CamelCase), temp.path().to_path_buf()).unwrap();
    let result = style_contract::run(&config).unwrap();
    let unused_messages: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "unused-class")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        unused_messages,
        vec!["Unused class 'passed'", "Unused class 'ready'"],
        "{:#?}",
        result.diagnostics
    );
    assert!(
        result
            .unused_symbols
            .iter()
            .all(|symbol| { symbol.name != "failed" && symbol.name != "blocked" })
    );
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
