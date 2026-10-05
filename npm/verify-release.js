#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const manifest = require(path.join(root, "package.json"));
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const cargoLock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
const lockVersion = cargoLock.match(/\[\[package\]\]\s+name = "style-contract"\s+version = "([^"]+)"/m)?.[1];

if (cargoVersion !== manifest.version || lockVersion !== manifest.version) {
  throw new Error(
    `release versions differ: package.json=${manifest.version}, Cargo.toml=${cargoVersion}, Cargo.lock=${lockVersion}`,
  );
}

for (const [name, version] of Object.entries(manifest.optionalDependencies ?? {})) {
  if (!name.startsWith("@style-contract/binary-") || version !== manifest.version) {
    throw new Error(`invalid platform dependency ${name}@${version}`);
  }
}

console.log(`release metadata is consistent for ${manifest.version}`);
