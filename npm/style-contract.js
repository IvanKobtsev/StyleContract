#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

function linuxLibc() {
  const report = process.report?.getReport?.();
  return report?.header?.glibcVersionRuntime ? "gnu" : "musl";
}

const platform = process.platform;
const arch = process.arch;
const suffix = platform === "linux" ? `${platform}-${arch}-${linuxLibc()}` : `${platform}-${arch}`;
const binaryPackage = `@style-contract/binary-${suffix}`;
let executable;

try {
  const packageJson = require.resolve(`${binaryPackage}/package.json`);
  executable = path.join(
    path.dirname(packageJson),
    platform === "win32" ? "style-contract.exe" : "style-contract",
  );
} catch (error) {
  console.error(
    `style-contract: no precompiled binary is installed for ${platform}/${arch}` +
      (platform === "linux" ? ` (${linuxLibc()})` : "") +
      `. Expected package ${binaryPackage}. Reinstall without omitting optional dependencies.`,
  );
  process.exit(2);
}

if (platform !== "win32") {
  try {
    const mode = fs.statSync(executable).mode;
    if ((mode & 0o111) === 0) fs.chmodSync(executable, mode | 0o111);
  } catch (error) {
    console.error(`style-contract: could not make the native binary executable: ${error.message}`);
    process.exit(2);
  }
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

