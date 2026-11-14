// `node scripts/release/set-version.mjs 26.1.0-alpha`: writes the one version into every field
// that holds it, then refreshes the lockfile. `src-tauri/tests/version_fields.rs` fails if they
// ever disagree.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { writeSetupVersionFields, writeVersionFields } from "./lib.mjs";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

export function setVersion(version, root = ROOT) {
  const paths = { packageJson: join(root, "package.json"), tauriConf: join(root, "src-tauri/tauri.conf.json"), cargoToml: join(root, "src-tauri/Cargo.toml") };
  const before = Object.fromEntries(Object.entries(paths).map(([k, p]) => [k, readFileSync(p, "utf8")]));
  const after = writeVersionFields(before, version);
  for (const [k, p] of Object.entries(paths)) writeFileSync(p, after[k]);
  const setup = { packageJson: join(root, "installer/package.json"), tauriConf: join(root, "installer/src-tauri/tauri.conf.json"), cargoToml: join(root, "installer/src-tauri/Cargo.toml") };
  const setupBefore = Object.fromEntries(Object.entries(setup).map(([k, p]) => [k, readFileSync(p, "utf8")]));
  const setupAfter = writeSetupVersionFields(setupBefore, version);
  for (const [k, p] of Object.entries(setup)) writeFileSync(p, setupAfter[k]);
  execFileSync("cargo", ["update", "--workspace", "--offline", "--manifest-path", join(root, "src-tauri/Cargo.toml")], { stdio: "pipe" });
  execFileSync("cargo", ["update", "-p", "lumen-setup", "--offline", "--manifest-path", join(root, "installer/src-tauri/Cargo.toml")], { stdio: "pipe" });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const version = process.argv[2];
  if (!version) {
    console.error("usage: node scripts/release/set-version.mjs 26.X.Y-alpha|beta|stable");
    process.exit(2);
  }
  setVersion(version);
  console.log(`Version set to ${version}.`);
}
