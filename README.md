# StyleContract

StyleContract is a fast, standalone Rust CLI that verifies the contract between TypeScript CSS Module references and CSS/SCSS declarations. It detects missing symbols, unused classes and `:export` keys, naming mismatches, and dynamic accesses that cannot be checked safely.

## Install

Build from source with a current stable Rust toolchain:

```console
cargo install --path .
```

## Use

Choose exactly one naming convention:

```console
style-contract --convention camel-case
style-contract --convention kebab-case
style-contract --convention camel-to-kebab
```

`camel-to-kebab` maps `styles.primaryButton` to `.primary-button`. `kebab-case` references use bracket notation such as `styles["primary-button"]`.

Available options:

```text
--source <PATH>             Source folder (default: ./src)
--exclude <PATH>            Excluded folder; repeat as needed
--include <PATH>            Re-include a folder below an exclusion
--ignore-exports            Ignore :export declarations
--rule <RULE:SEVERITY>      Set a rule to error, warning, or off
--tsconfig <PATH>           tsconfig used for paths aliases (default: ./tsconfig.json)
```

Paths are resolved from the working directory. Include and exclude folders must exist beneath the source folder. A tsconfig is optional when no aliases are needed.

Supported rules:

- `no-missing-symbols`
- `no-unused-classes`
- `no-unused-exports`
- `naming-convention`
- `no-dynamic-references`

All rules are errors except `no-dynamic-references`, which is a warning. For example:

```console
style-contract --convention camel-to-kebab \
  --exclude ./src/generated \
  --include ./src/generated/checked \
  --rule no-unused-exports:warning
```

Diagnostics follow `path:line:column severity rule message`. Exit code `0` means no errors, `1` means rule errors were found, and `2` means configuration or analysis failed.

## Static-analysis boundary

The MVP supports `.ts`, `.tsx`, `.css`, and `.scss`, default or namespace CSS Module imports, relative imports, tsconfig `baseUrl`/`paths`, dot and literal bracket access, and static destructuring. It recognizes literal class selectors, nested literal SCSS selectors, and `:export` blocks.

Dynamic Sass-generated selectors and computed accesses such as `styles[name]` produce warnings. Because their complete usage cannot be proven, unused-symbol findings are suppressed for that stylesheet. Sass evaluation, `composes`, and explicit `:global`/`:local` semantics are intentionally outside the MVP.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

