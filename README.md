# StyleContract

StyleContract is a fast, standalone Rust CLI that verifies the contract between TypeScript CSS Module references and CSS/SCSS declarations. It detects missing symbols, unused classes and `:export` keys, empty style rules, unsafe module-to-module stylesheet imports, naming mismatches, and dynamic accesses that cannot be checked safely.

## Install

Install through npm. The matching precompiled executable is selected automatically:

```console
npm install --save-dev style-contract
```

Precompiled packages are available for Windows, macOS, and Linux on x64 and ARM64. Linux supports both glibc and musl. Package managers that omit optional dependencies are not supported because the native executable is delivered as a platform-specific optional dependency.

The platform packages use names such as `@style-contract/binary-win32-x64`. They are implementation details and should not be installed directly.

It can then be run from package scripts or directly with `npx`:

```console
npx style-contract --convention camel-to-kebab
```

Alternatively, build and install directly from the Rust source:

```console
cargo install --path .
```

## Use

Choose exactly one naming convention on the command line or in a config file:

```console
style-contract --convention camel-case
style-contract --convention kebab-case
style-contract --convention camel-to-kebab
```

`camel-case` accepts both lower-camel names such as `primaryButton` and capitalized names such as `ComponentRoot`. `camel-to-kebab` continues to require lower-camel TypeScript references and maps `styles.primaryButton` to `.primary-button`. `kebab-case` references use bracket notation such as `styles["primary-button"]`.

Available options:

```text
--source <PATH>             Source folder (default: ./src)
--output <minimal|rich>     Output style (default: rich)
--config <PATH>             JSON5 configuration file (auto: ./style-contract.json)
--exclude <PATH>            Excluded folder; repeat as needed
--include <PATH>            Re-include a folder below an exclusion
--ignore-exports[=BOOL]     Enable or explicitly disable :export checks
--rule <RULE:SEVERITY>      Set a rule to error, warning, or off
--tsconfig <PATH>           tsconfig used for paths aliases (default: ./tsconfig.json)
```

CLI paths are resolved from the working directory. Paths in a config file are resolved from that file's directory. Include and exclude folders must exist beneath the source folder. A tsconfig is optional when no aliases are needed.

With no explicit `--config`, StyleContract automatically loads `./style-contract.json` when it exists. Use `--config` to select another file. JSON5 comments and trailing commas are supported regardless of the filename extension:

```json5
{
  convention: "camel-to-kebab",
  output: "rich",
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
    "module-to-module-import": "warning",
  },
}
```

Explicit CLI values replace matching config values. Supplying any CLI entries for `--exclude`, `--include`, or `--rule` replaces that configured collection. `--ignore-exports=false` can disable a value enabled by the config.

Supported rules:

- `missing-symbol`
- `unused-class`
- `unused-dependent-class` (off by default)
- `unused-export`
- `naming-convention-local`
- `naming-convention-global`
- `dynamic-reference`
- `empty-rule`
- `module-to-module-import`

All enabled rules are errors except `naming-convention-global`, `dynamic-reference`, `empty-rule`, and `module-to-module-import`, which are warnings. `unused-dependent-class` is disabled by default; when enabled, it verifies that classes declared in same-element compound selectors are used together with a complete prerequisite class path in one JSX `className` expression or `clsx(...)` call. `empty-rule` reports selector blocks that contain only whitespace or comments; empty `:export` and structural at-rule blocks are not reported. `module-to-module-import` reports `@use`, `@forward`, and `@import` dependencies from one `.module.css` or `.module.scss` file to another. Imports from ordinary non-module stylesheets remain allowed. The warning exists because Sass merges the imported module's classes into the importing module's generated class map, which can expose unexpected classes and compound through transitive import chains.

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

Rich output uses compiler-style diagnostics, terminal colors, linked-looking source locations, compact per-source error/warning totals, and a final total across all affected files. Use `--output minimal` for the original single-line format. Colors are disabled when output is redirected or the `NO_COLOR` environment variable is set.

## Static-analysis boundary

The MVP supports `.ts`, `.tsx`, `.css`, and `.scss`, default or namespace CSS Module imports, relative imports, tsconfig `baseUrl`/`paths`, dot and literal bracket access, and static destructuring. It recognizes literal class selectors, nested literal SCSS selectors, empty selector blocks, `:export` blocks, CSS Modules `:global`/`:local` functions and blocks, Sass `@use`/`@forward`/`@import` dependencies, Sass `@extend`, and same-file or relative-file CSS Modules `composes` references.

Classes in `.module.css` and `.module.scss` are local by default. Classes in other stylesheets are global by default. Global classes are checked against the warning-level global convention rule, but are not CSS Module exports and therefore do not participate in missing or unused checks.

Dynamic Sass-generated selectors and computed accesses such as `styles[name]` produce warnings. Because their complete usage cannot be proven, unused-symbol findings are suppressed for that stylesheet. StyleContract detects module-to-module Sass dependencies but does not evaluate Sass or flatten imported classes into the importing module's symbol table. Sass evaluation and `composes` evaluation remain outside the MVP.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

## Release

Releases are published from version tags. Before the first release, create or claim the `@style-contract` organization on npm, grant the publisher access to it, and add an npm automation token to the GitHub repository as the `NPM_TOKEN` Actions secret.

Keep the version in `package.json`, `Cargo.toml`, `Cargo.lock`, and every platform package dependency identical. The release verifier enforces this. Push a tag matching that version, such as `v0.7.2`; the release workflow builds and publishes all eight platform packages before publishing `style-contract`.

