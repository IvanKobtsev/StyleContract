# StyleContract

StyleContract is a fast, standalone Rust CLI that verifies the contract between TypeScript CSS Module references and CSS/SCSS declarations. It detects missing symbols, unused classes and `:export` keys, empty style rules, class leaks from Sass imports, optional naming mismatches, and dynamic accesses that cannot be checked safely.

## Install

Install through npm. The matching precompiled executable is selected automatically:

```console
npm install --save-dev style-contract
```

Precompiled packages are available for Windows, macOS, and Linux on x64 and ARM64. Linux supports both glibc and musl. Package managers that omit optional dependencies are not supported because the native executable is delivered as a platform-specific optional dependency.

The platform packages use names such as `@style-contract/binary-win32-x64`. They are implementation details and should not be installed directly.

It can then be run from package scripts or directly with `npx`:

```console
npx style-contract
```

Alternatively, build and install directly from the Rust source:

```console
cargo install --path .
```

## Use

Naming enforcement is optional. With no convention, StyleContract uses `none`: it emits no naming diagnostics and treats `styles.primaryButton` as a reference to both `.primaryButton` and `.primary-button`. This is deliberately not convention detection.

Choose an explicit convention on the command line or in a config file when a project should enforce one:

```console
style-contract --convention camel-case
style-contract --convention kebab-case
style-contract --convention camel-to-kebab
style-contract --convention none
```

`camel-case` accepts both lower-camel names such as `primaryButton` and capitalized names such as `ComponentRoot`. `camel-to-kebab` requires lower-camel TypeScript references and maps `styles.primaryButton` to `.primary-button`. `kebab-case` references use bracket notation such as `styles["primary-button"]`. In `none`, kebab references remain exact: `styles["primary-button"]` does not also reference `.primaryButton`.

Available options:

```text
--convention <none|camel-case|kebab-case|camel-to-kebab>
                            Naming convention (default: none)
--source <PATH>             Source folder (default: ./src)
--output <minimal|rich>     Output style (default: rich)
--print-only-errors[=BOOL]  Print errors without printing warnings
--config <PATH>             JSON5 configuration file (auto: ./style-contract.json)
--exclude <PATH>            Excluded folder; repeat as needed
--include <PATH>            Re-include a folder below an exclusion
--ignore-exports[=BOOL]     Enable or explicitly disable :export checks
--rule <RULE:SEVERITY>      Set a rule to error, warning, or off
--tsconfig <PATH>           tsconfig used for paths aliases (default: ./tsconfig.json)
```

CLI paths are resolved from the working directory. Paths in a config file are resolved from that file's directory. Include and exclude folders must exist beneath the source folder. A tsconfig is optional when no aliases are needed. Each tsconfig `paths` replacement is resolved relative to the config that declares it, including mappings inherited through `extends`. StyleContract does not use `baseUrl`; migrate stylesheet aliases that rely on it to explicit `paths` mappings.

With no explicit `--config`, StyleContract automatically loads `./style-contract.json` when it exists. Use `--config` to select another file. JSON5 comments and trailing commas are supported regardless of the filename extension:

```json5
{
  convention: "camel-to-kebab",
  output: "rich",
  printOnlyErrors: false,
  source: "./src",
  exclude: ["./src/generated"],
  include: [],
  ignoreExports: false,
  tsconfig: "./tsconfig.json",
  rules: {
    "unused-export": "warning",
    "unused-dependent-class": "warning",
    "naming-convention-global": "off",
    "empty-rule": "warning",
    "class-leak": "warning",
  },
}
```

Explicit CLI values replace matching config values. Supplying any CLI entries for `--exclude`, `--include`, or `--rule` replaces that configured collection. `--ignore-exports=false` and `--print-only-errors=false` can disable values enabled by the config.

Supported rules:

- `missing-symbol`
- `unused-class`
- `unused-dependent-class` (off by default)
- `unused-export`
- `naming-convention-local`
- `naming-convention-global`
- `dynamic-reference`
- `empty-rule`
- `class-leak`

All enabled rules are errors except `naming-convention-local`, `naming-convention-global`, `dynamic-reference`, `empty-rule`, and `class-leak`, which are warnings. Naming-convention rules do not emit findings in `none` mode. `unused-dependent-class` is disabled by default; when enabled, it verifies that classes declared in same-element compound selectors are used together with a complete prerequisite class path in one JSX `className` expression or `clsx(...)` call. `empty-rule` reports selector blocks that contain only whitespace or comments; empty `:export` and structural at-rule blocks are not reported. `class-leak` reports a CSS Module's direct `@use`, `@forward`, and `@import` dependencies when the imported file declares classes. This includes ordinary stylesheets as well as other CSS Modules, because those classes are emitted into the importing module. Keep shared Sass dependencies class-free; when another CSS Module's classes are needed in TypeScript, import that module independently from the TS or TSX file.

### Targeted suppressions

When a deliberate abstraction cannot be followed statically, use a targeted suppression comment. Directives work in TypeScript, CSS, and SCSS and accept canonical rule names separated by spaces or commas. Omit the rules to suppress every rule.

```scss
/* @sc-ignore unused-dependent-class */
&.highlighted { color: yellow; }
```

```ts
// @sc-ignore dynamic-reference
return styles[name];
```

`@sc-ignore` applies to the next physical line. Use a nested block when several lines need the same suppression:

```scss
/* @sc-ignore-start unused-dependent-class */
.root.active { color: red; }
.root.selected { color: blue; }
/* @sc-ignore-end */
```

Blocks apply after the start marker through the line before the matching end marker. End markers close the nearest open block. Invalid rule names and unmatched markers are errors. Suppressing `unused-dependent-class` on a declaration makes that declaration participate in usage analysis and navigation as an ordinary class.

Previous `no-*` rule IDs remain accepted as configuration aliases, and the legacy `naming-convention` override remains available as an alias for both convention rules. For example:

```console
style-contract --convention camel-to-kebab \
  --exclude ./src/generated \
  --include ./src/generated/checked \
  --rule unused-export:warning
```

Diagnostics follow `path:line:column severity rule message`. Exit code `0` means no errors, `1` means rule errors were found, and `2` means configuration or analysis failed.

Rich output uses compiler-style diagnostics, terminal colors, linked-looking source locations, compact per-source error/warning totals, and a final total across all affected files. Use `--output minimal` for the original single-line format. Use `--print-only-errors` to hide warnings from either report without disabling their analysis. Colors are disabled when output is redirected or the `NO_COLOR` environment variable is set.

## Static-analysis boundary

The MVP supports `.ts`, `.tsx`, `.css`, and `.scss`, default or namespace CSS Module imports, relative imports, tsconfig `paths`, dot and literal bracket access, and static destructuring. `baseUrl` resolution is not supported; define non-relative stylesheet aliases with `compilerOptions.paths`. It recognizes literal class selectors, nested literal SCSS selectors, empty selector blocks, `:export` blocks, CSS Modules `:global`/`:local` functions and blocks, Sass `@use`/`@forward`/`@import` dependencies, Sass `@extend`, and same-file or relative-file CSS Modules `composes` references.

Classes in `.module.css` and `.module.scss` are local by default. Classes in other stylesheets are global by default. With an explicit convention, global classes are checked against the warning-level global convention rule, but are not CSS Module exports and therefore do not participate in missing or unused checks.

Dynamic Sass-generated selectors and computed accesses such as `styles[name]` produce warnings. Because their complete usage cannot be proven, unused-symbol findings are suppressed for that stylesheet. StyleContract detects direct class leaks from resolvable Sass dependencies but does not evaluate Sass, follow transitive Sass imports, or flatten imported classes into the importing module's symbol table. Sass evaluation and `composes` evaluation remain outside the MVP.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

## Release

Releases are published from version tags. Before the first release, create or claim the `@style-contract` organization on npm, grant the publisher access to it, and add an npm automation token to the GitHub repository as the `NPM_TOKEN` Actions secret.

Keep the version in `package.json`, `Cargo.toml`, `Cargo.lock`, and every platform package dependency identical. The release verifier enforces this. Push a tag matching that version, such as `v0.9.0`; the release workflow builds and publishes all eight platform packages before publishing `style-contract`.

