#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const [source, output, packageName, os, cpu, libc = ""] = process.argv.slice(2);
if (!source || !output || !packageName || !os || !cpu) {
  console.error(
    "usage: create-platform-package <binary> <output> <package-name> <os> <cpu> [libc]",
  );
  process.exit(2);
}

const rootPackage = require(path.resolve(__dirname, "..", "package.json"));
const cargoToml = fs.readFileSync(path.resolve(__dirname, "..", "Cargo.toml"), "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (cargoVersion !== rootPackage.version) {
  throw new Error(`package.json (${rootPackage.version}) and Cargo.toml (${cargoVersion}) versions differ`);
}

fs.mkdirSync(output, { recursive: true });
const executableName = os === "win32" ? "style-contract.exe" : "style-contract";
fs.copyFileSync(source, path.join(output, executableName));
if (os !== "win32") fs.chmodSync(path.join(output, executableName), 0o755);

const manifest = {
  name: packageName,
  version: rootPackage.version,
  description: `Precompiled StyleContract binary for ${os}/${cpu}${libc ? `/${libc}` : ""}`,
  repository: rootPackage.repository,
  license: rootPackage.license,
  os: [os],
  cpu: [cpu],
  files: [executableName],
  publishConfig: { access: "public" },
};
if (libc) manifest.libc = libc === "gnu" ? "glibc" : libc;

fs.writeFileSync(path.join(output, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
fs.copyFileSync(path.resolve(__dirname, "..", "README.md"), path.join(output, "README.md"));
fs.copyFileSync(path.resolve(__dirname, "..", "LICENSE"), path.join(output, "LICENSE"));
