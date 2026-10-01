#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const path = require("node:path");

const packageRoot = path.resolve(__dirname, "..");
const executable = path.join(
  packageRoot,
  "target",
  "release",
  process.platform === "win32" ? "style-contract.exe" : "style-contract",
);

if (!existsSync(executable)) {
  console.error(
    "style-contract: the Rust binary is missing. Reinstall the package with npm scripts enabled and ensure Cargo is installed.",
  );
  process.exit(2);
}

const result = spawnSync(executable, process.argv.slice(2), {
  cwd: process.cwd(),
  stdio: "inherit",
});

if (result.error) {
  console.error(`style-contract: failed to start: ${result.error.message}`);
  process.exit(2);
}

process.exit(result.status ?? 2);

