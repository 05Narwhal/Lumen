// The facts the gates judge (Round 17): read from git, cargo, the sidecars, the docs and GitHub.
// Nothing here changes the repository except `yarn run licenses` regenerating notices.json, which
// is how "notices regenerated" is checked. Secrets are only tested for presence.
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { compareVersions, parseVersion } from "./lib.mjs";
import { rawlerDependents } from "./gates.mjs";
import { REPO } from "./publish.mjs";

const TRIPLE = "x86_64-pc-windows-msvc";
export const TEST_OVERRIDES = ["LUMEN_BAKED_APP_DATA_DIR", "LUMEN_BAKED_LIBRARY_DIR", "LUMEN_UPDATE_TEST_BASE", "LUMEN_SETUP_PRODUCT", "LUMEN_SETUP_IDENTIFIER"];

// A shell only for yarn, a .cmd on Windows. Through a shell, arguments are not quoted, so a path
// with a space (the repo folder may have one) splits in two and the call fails.
const run = (cmd, args, cwd) => {
  const r = spawnSync(cmd, args, { cwd, encoding: "utf8", shell: process.platform === "win32" && cmd === "yarn", maxBuffer: 1 << 28 });
  return { ok: r.status === 0, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

/** Every file in `dir` with its SHA-256. */
export function hashFolder(dir) {
  if (!existsSync(dir)) return {};
  return Object.fromEntries(readdirSync(dir).filter((n) => statSync(join(dir, n)).isFile()).map((n) => [n, sha256(join(dir, n))]));
}

/** The newest published version in the releases repo; `undefined` when gh can't see the repo. */
function previousRelease() {
  const r = run("gh", ["release", "list", "--repo", REPO, "--limit", "200", "--json", "tagName"]);
  if (!r.ok) return undefined;
  const versions = JSON.parse(r.out)
    .map((x) => x.tagName.replace(/^v/, ""))
    .filter((v) => {
      try {
        parseVersion(v);
        return true;
      } catch {
        return false;
      }
    });
  return versions.sort(compareVersions).pop() ?? null;
}

export function collectFacts(root, { version, skipChecks = false, sourceDir }) {
  const conf = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
  const bin = (name) => join(root, "src-tauri/binaries", `${name}-${TRIPLE}.exe`);
  const ffmpegPath = bin("ffmpeg");
  const previous = previousRelease();
  const facts = {
    branch: run("git", ["rev-parse", "--abbrev-ref", "HEAD"], root).out.trim(),
    clean: run("git", ["status", "--porcelain"], root).out.trim() === "",
    changelogVersions: [...readFileSync(join(root, "docs/CHANGELOG.md"), "utf8").matchAll(/^## (\S+)/gm)].map((m) => m[1]),
    versionFields: JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version,
    previousVersion: previous ?? null,
    releasesRepo: previous !== undefined,
    sidecars: { ffmpeg: existsSync(ffmpegPath), ffprobe: existsSync(bin("ffprobe")), exiftool: existsSync(bin("exiftool")) && existsSync(join(root, "src-tauri/binaries/exiftool_files")) },
    pubkey: conf.plugins?.updater?.pubkey ?? "",
    privateKeyInEnv: !!process.env.TAURI_SIGNING_PRIVATE_KEY,
    testOverrides: TEST_OVERRIDES.filter((k) => process.env[k]),
    libraryCheck: { ok: false, why: "not run yet: it needs the built candidate" },
    sources: JSON.parse(readFileSync(join(root, "scripts/release/sources.json"), "utf8")),
    gates: JSON.parse(readFileSync(join(root, "scripts/release/gates.json"), "utf8")),
    sourceBundle: hashFolder(sourceDir),
  };
  const notices = run("yarn", ["-s", "run", "licenses"], root);
  facts.noticesFresh = notices.ok && run("git", ["status", "--porcelain", "--", "src/assets/licenses/notices.json"], root).out.trim() === "";
  const tree = run("cargo", ["tree", "-i", "rawler", "-e", "normal", "--depth", "1", "--prefix", "none"], join(root, "src-tauri"));
  const resources = conf.bundle?.resources ?? [];
  facts.rawler = { dependents: rawlerDependents(tree.out), resources: Array.isArray(resources) ? resources : Object.keys(resources) };
  facts.ffmpeg = facts.sidecars.ffmpeg
    ? { versionOutput: execFileSync(ffmpegPath, ["-hide_banner", "-version"], { encoding: "utf8" }), sidecarSha256: sha256(ffmpegPath) }
    : { versionOutput: null, sidecarSha256: null };
  if (skipChecks) {
    facts.checks = { ok: false, failed: ["skipped (--skip-checks)"] };
  } else {
    const tauri = join(root, "src-tauri");
    const checks = [
      ["cargo test --workspace", "cargo", ["test", "--workspace"], tauri],
      ["cargo clippy", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], tauri],
      ["yarn typecheck", "yarn", ["typecheck"], root],
      ["yarn test", "yarn", ["test"], root],
    ];
    const failed = checks.filter(([, cmd, args, cwd]) => !run(cmd, args, cwd).ok).map(([name]) => name);
    facts.checks = { ok: failed.length === 0, failed };
  }
  void version;
  return facts;
}

/** The release check on the built candidate: a temp copy of a real library opens and migrates. */
export function libraryCheck(exe, library) {
  if (!library) return { ok: false, why: "LUMEN_RELEASE_LIBRARY names no library to copy" };
  if (!existsSync(exe)) return { ok: false, why: `${exe} was not built` };
  const r = spawnSync(exe, ["--check-library", library], { encoding: "utf8" });
  try {
    const out = JSON.parse(r.stdout.trim().split(/\r?\n/).pop());
    return out.ok ? { ok: true, from: out.from, to: out.to } : { ok: false, why: out.why };
  } catch {
    return { ok: false, why: `no answer from --check-library (exit ${r.status})` };
  }
}
