// Lumen Setup's part of the release build (Round 18 A2, phase 7). The order is fixed by one fact:
// the payload has to be inside the signed file, never appended after signing.
//
//   1. the slim Setup (no payload)  ->  signed with signtool when LUMEN_SIGN_CERT_SHA1 is set
//   2. `tauri build --bundles nsis` with the slim Setup as a bundle resource landing at
//      $INSTDIR\lumen-setup.exe (the hook in installer-hooks.nsh makes it the uninstaller); the
//      installer is signed by Tauri's signCommand *during* the build, because the updater's .sig is
//      taken over the finished file and signing afterwards would invalidate it
//   3. the real installed size is measured and baked into the full Setup
//   4. the full Setup, with that installer embedded (feature `payload`) -> signed -> written beside
//      the installer as Lumen-Setup-<version>.exe
//
// Without a certificate everything builds unsigned and the release says SmartScreen will warn.
// The decisions are pure functions so they are tested without a build; `buildInstallers` is the
// orchestration the release workflow runs through this file's CLI (`node setup-build.mjs <version>`).
// The updater's minisign key comes from the environment only (`updaterKeyFromEnv`).
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative } from "node:path";
import { pathToFileURL } from "node:url";
import { parseVersion } from "./lib.mjs";
import { ROOT } from "./set-version.mjs";

export const TRIPLE = "x86_64-pc-windows-msvc";
/** Where the slim Setup lands inside the install folder. Installer-hooks.nsh looks for this exact name. */
export const SLIM_TARGET = "lumen-setup.exe";
/** uninstall.exe, which NSIS writes next to the program (81.7 KB measured on 2026-11-06). */
const UNINSTALLER_BYTES = 128 * 1024;
const DEFAULT_TIMESTAMP = "http://timestamp.digicert.com";

/** RFC 7396 merge, as `tauri build` merges `tauri.windows.conf.json` and `--config` into the base. */
export function mergePatch(base, patch) {
  if (patch === null || typeof patch !== "object" || Array.isArray(patch)) return patch;
  const out = base && typeof base === "object" && !Array.isArray(base) ? { ...base } : {};
  for (const [k, v] of Object.entries(patch)) {
    if (v === null) delete out[k];
    else out[k] = mergePatch(out[k], v);
  }
  return out;
}

/** The certificate Julian names: the SHA-1 thumbprint of one in his certificate store. No secret in the environment. */
export function signingFromEnv(env) {
  const thumbprint = (env.LUMEN_SIGN_CERT_SHA1 ?? "").replace(/\s+/g, "");
  if (!thumbprint) return null;
  if (!/^[0-9a-fA-F]{40}$/.test(thumbprint)) throw new Error("LUMEN_SIGN_CERT_SHA1 must be the 40-hex-digit SHA-1 thumbprint of a certificate in your certificate store.");
  return { thumbprint, timestamp: env.LUMEN_SIGN_TIMESTAMP_URL || DEFAULT_TIMESTAMP };
}

/** `signtool sign` arguments for one file (SHA-256, RFC 3161 timestamp). */
export function signtoolArgs(sign, file) {
  return ["sign", "/sha1", sign.thumbprint, "/fd", "SHA256", "/tr", sign.timestamp, "/td", "SHA256", file];
}

/** Tauri's `bundle.windows.signCommand`: it substitutes `%1` with each file it signs. */
export function tauriSignCommand(sign, signtool = "signtool") {
  return { cmd: signtool, args: signtoolArgs(sign, "%1") };
}

/** The `--config` overlay for the NSIS build: updater artefacts, the slim Setup as a resource, signing. */
export function nsisOverlay({ slimPath, sign, signtool }) {
  const bundle = { createUpdaterArtifacts: true, resources: { [slimPath]: SLIM_TARGET } };
  if (sign) bundle.windows = { signCommand: tauriSignCommand(sign, signtool) };
  return { bundle };
}

/**
 * The slim Setup's `resources` key: relative to `src-tauri`, with forward slashes. tauri-build
 * drops the drive of an absolute Windows path (`C:\x` became `/x`, phase 8), so a file on another
 * drive than the repo cannot be a resource.
 */
export function resourceKey(srcTauri, file) {
  const rel = relative(srcTauri, file);
  if (!rel || isAbsolute(rel)) throw new Error(`The slim Setup must be on the same drive as the repo to be bundled (${file}).`);
  return rel.replaceAll("\\", "/");
}

/** The files and folders a bundle `resources` key stands for, as sizes: `dir/*`, `dir/` (a folder) or a file. */
function resourceBytes(srcTauri, key) {
  const wildcard = key.endsWith("/*");
  const path = join(srcTauri, wildcard ? key.slice(0, -2) : key.replace(/[\\/]$/, ""));
  if (!existsSync(path)) throw new Error(`bundle resource ${key} does not exist (${path})`);
  return treeBytes(path);
}

/** Total bytes of a file or of every file under a folder. */
export function treeBytes(path) {
  const s = statSync(path);
  if (!s.isDirectory()) return s.size;
  return readdirSync(path).reduce((n, name) => n + treeBytes(join(path, name)), 0);
}

/**
 * What the NSIS installer lays down, from the merged Tauri config: Lumen itself, the sidecars
 * (`externalBin`, installed without the target triple), every resource, the slim Setup and the
 * uninstaller. The 5 % headroom is added by Setup (`need_bytes`), not here.
 */
export function installedBytes({ srcTauri, config, lumenExe, slimPath, slimKey = slimPath }) {
  const bundle = config.bundle ?? {};
  let total = statSync(lumenExe).size + UNINSTALLER_BYTES + statSync(slimPath).size;
  for (const name of bundle.externalBin ?? []) total += statSync(join(srcTauri, `${name}-${TRIPLE}.exe`)).size;
  for (const key of Object.keys(bundle.resources ?? {})) {
    if (key === slimKey) continue;
    total += resourceBytes(srcTauri, key);
  }
  return total;
}

/** The base config with the platform file merged in, as `tauri build` reads it (no `--config` overlay). */
export function windowsConfig(srcTauri) {
  const read = (n) => JSON.parse(readFileSync(join(srcTauri, n), "utf8"));
  return mergePatch(read("tauri.conf.json"), read("tauri.windows.conf.json"));
}

/**
 * The updater's key must come from the environment as the key's own text (the GitHub secret CI sets),
 * never as a path to a file on someone's PC: a build that works only on one machine is the local
 * release path this replaced. The key is base64, so anything with a `.`, `:`, `\` or `~` is a path.
 * Tauri reads `TAURI_SIGNING_PRIVATE_KEY` itself from the environment `run` hands on; this only
 * refuses early, before ten minutes of building end in "no private key".
 */
export function updaterKeyFromEnv(env) {
  const key = (env.TAURI_SIGNING_PRIVATE_KEY ?? "").trim();
  if (!key) throw new Error("TAURI_SIGNING_PRIVATE_KEY is not set: the updater artefacts cannot be signed. CI sets it from the repository secret.");
  if (!/^[A-Za-z0-9+/=\s]+$/.test(key)) throw new Error("TAURI_SIGNING_PRIVATE_KEY must hold the key's text, not a file path.");
  if (env.TAURI_SIGNING_PRIVATE_KEY_PATH) throw new Error("TAURI_SIGNING_PRIVATE_KEY_PATH is set: the key comes from TAURI_SIGNING_PRIVATE_KEY only, never from a file on a PC.");
  return true;
}

/**
 * The files a release uploads, all in the folder Tauri writes the NSIS installer to: the installer
 * (the only file the updater installs), its `.sig` and the full Setup. `uploadGlobs` are what the
 * workflow's upload step matches inside that folder.
 */
export function collectArtifacts(nsisDir, version) {
  const names = readdirSync(nsisDir);
  const installerName = names.find((n) => n.includes(`_${version}_`) && n.endsWith("-setup.exe"));
  if (!installerName) throw new Error(`No installer for ${version} in ${nsisDir}.`);
  const sigName = `${installerName}.sig`;
  if (!names.includes(sigName)) throw new Error(`${sigName} is missing: without its .sig the updater could never take ${installerName}. Was the updater key set when it was built?`);
  const fullName = `Lumen-Setup-${version}.exe`;
  if (!names.includes(fullName)) throw new Error(`${fullName} is missing in ${nsisDir}.`);
  return {
    installer: join(nsisDir, installerName),
    installerName,
    sig: join(nsisDir, sigName),
    full: join(nsisDir, fullName),
    fullName,
    uploadGlobs: [`*_${version}_*-setup.exe*`, fullName],
  };
}

/** What the `setup-sizes` gate reads (`preflight.mjs after --sizes`). */
export const sizesJson = ({ sizes }) => ({ nsis: sizes.nsis, full: sizes.full, slim: sizes.slim });

/** `signtool.exe` on PATH, else the newest Windows SDK's x64 one; null when there is none. */
export function findSigntool(env = process.env) {
  for (const dir of (env.PATH ?? env.Path ?? "").split(";")) {
    if (dir && existsSync(join(dir, "signtool.exe"))) return join(dir, "signtool.exe");
  }
  const kits = join(env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "Windows Kits", "10", "bin");
  if (!existsSync(kits)) return null;
  const versions = readdirSync(kits).filter((n) => /^10\./.test(n)).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  for (const v of versions) {
    const p = join(kits, v, "x64", "signtool.exe");
    if (existsSync(p)) return p;
  }
  return null;
}

function run(cmd, args, { cwd, env, label }) {
  const r = spawnSync(cmd, args, { cwd, env: { ...process.env, ...env }, stdio: "inherit" });
  if (r.status !== 0) throw new Error(`${label} failed (${r.status ?? r.error?.message}).`);
}

/** The `tauri build` arguments: our overlay first, then any test-identity overlay files, in order. */
export function nsisBuildArgs(overlay, extraConfigs = []) {
  return ["build", "--bundles", "nsis", "--config", overlay, ...extraConfigs.flatMap((c) => ["--config", c])];
}

/** Lumen's built exe: `mainBinaryName` when a config sets it (a test identity), else `lumen`. */
export function lumenExeName(config) {
  return `${config.mainBinaryName ?? "lumen"}.exe`;
}

/**
 * Builds the NSIS installer and both Setups. Returns the paths and sizes the gate and the upload
 * need. Throws with what failed; nothing is published from here. `extraConfigs` are overlay files
 * merged after ours (phase 8's *Lumen Test* identity); a release passes none.
 */
export function buildInstallers({ root, version, out, env = process.env, log = console.log, extraConfigs = [], requireUpdaterKey = true }) {
  if (requireUpdaterKey) updaterKeyFromEnv(env);
  const srcTauri = join(root, "src-tauri");
  const setupDir = join(root, "installer/src-tauri");
  const setupTarget = join(setupDir, "target/release/lumen-setup.exe");
  const sign = signingFromEnv(env);
  const signtool = sign ? findSigntool(env) : null;
  if (sign && !signtool) throw new Error("LUMEN_SIGN_CERT_SHA1 is set but signtool.exe was not found (install the Windows SDK or put it on PATH).");
  if (!sign) log("No certificate (LUMEN_SIGN_CERT_SHA1 is not set): building unsigned. Windows SmartScreen will warn when people open Lumen-Setup and the installer.");
  const signFile = (file) => run(signtool, signtoolArgs(sign, file), { cwd: root, label: `Signing ${file}` });
  mkdirSync(out, { recursive: true });

  // Test identities and a stray payload never reach a release (the no-test-overrides gate checks the first).
  const clean = { LUMEN_SETUP_PAYLOAD: "", LUMEN_SETUP_INSTALLED_BYTES: "" };
  const cargoSetup = (extra, label) =>
    run("cargo", ["build", "--release", "--manifest-path", join(setupDir, "Cargo.toml"), "--features", `tauri/custom-protocol${extra.features ? `,${extra.features}` : ""}`], {
      cwd: root,
      env: { ...clean, ...extra.env },
      label,
    });

  log("Building Lumen Setup's window…");
  run(process.execPath, [join(root, "node_modules/vite/bin/vite.js"), "build", "--config", join(root, "installer/vite.config.ts")], { cwd: root, label: "Setup's interface build" });

  log("Building the slim Setup (the uninstaller)…");
  cargoSetup({}, "The slim Setup build");
  const slim = join(out, SLIM_TARGET);
  copyFileSync(setupTarget, slim);
  if (sign) signFile(slim);

  log("Building the signed NSIS installer with the slim Setup inside…");
  const overlay = join(out, "nsis-overlay.json");
  const slimKey = resourceKey(srcTauri, slim);
  writeFileSync(overlay, JSON.stringify(nsisOverlay({ slimPath: slimKey, sign, signtool }), null, 2));
  run(process.execPath, [join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...nsisBuildArgs(overlay, extraConfigs)], { cwd: root, label: "The NSIS build" });
  const nsisDir = join(srcTauri, "target/release/bundle/nsis");
  const installerName = readdirSync(nsisDir).find((n) => n.includes(`_${version}_`) && n.endsWith("-setup.exe"));
  if (!installerName) throw new Error(`No installer for ${version} in ${nsisDir}.`);
  const installer = join(nsisDir, installerName);

  const config = extraConfigs.reduce(
    (c, file) => mergePatch(c, JSON.parse(readFileSync(file, "utf8"))),
    mergePatch(windowsConfig(srcTauri), nsisOverlay({ slimPath: slimKey, sign: null })),
  );
  const baked = installedBytes({ srcTauri, config, lumenExe: join(srcTauri, "target/release", lumenExeName(config)), slimPath: slim, slimKey });
  log(`Lumen installs about ${(baked / 1048576).toFixed(0)} MB; baking that into the full Setup.`);

  log("Building the full Setup with the installer inside…");
  cargoSetup({ features: "payload", env: { LUMEN_SETUP_PAYLOAD: installer, LUMEN_SETUP_INSTALLED_BYTES: String(baked) } }, "The full Setup build");
  const fullName = `Lumen-Setup-${version}.exe`;
  const full = join(nsisDir, fullName);
  mkdirSync(dirname(full), { recursive: true });
  copyFileSync(setupTarget, full);
  if (sign) signFile(full);

  const artifacts = collectArtifacts(nsisDir, version);
  return {
    ...artifacts,
    slim,
    signed: !!sign,
    installedBytes: baked,
    sizes: { nsis: statSync(installer).size, full: statSync(full).size, slim: statSync(slim).size },
  };
}

/**
 * `node scripts/release/setup-build.mjs <version>`: the Windows release job's build step. Writes
 * `dist-release/<version>/sizes.json` (for `preflight.mjs after`) and `built.json` (the files).
 */
function main([version]) {
  parseVersion(version ?? "");
  const out = join(ROOT, "dist-release", version);
  const built = buildInstallers({ root: ROOT, version, out });
  mkdirSync(out, { recursive: true });
  writeFileSync(join(out, "sizes.json"), `${JSON.stringify(sizesJson(built), null, 2)}\n`);
  writeFileSync(join(out, "built.json"), `${JSON.stringify({ installer: built.installer, sig: built.sig, full: built.full, uploadGlobs: built.uploadGlobs, signed: built.signed, sizes: built.sizes }, null, 2)}\n`);
  console.log(`Built ${built.installerName} and ${built.fullName}; sizes in ${join(out, "sizes.json")}.`);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
