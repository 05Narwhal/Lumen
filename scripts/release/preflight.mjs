// The release's preflight (release simplification, phase 2). The `preflight` job in
// .github/workflows/release.yml runs it before anything is built, and once more after the build
// for the two gates that need the built candidate. The rules are pure functions of facts, so they
// are tested without git, cargo or GitHub; the CLI that gathers the facts is below them.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { ffmpegGate, RAW_LIBRARY, rawlerDependents, setupSizeGate, sourceBundleGate, tagRefGate, TEST_OVERRIDES } from "./gates.mjs";
import { isNextVersion, isSecurityRelease, parseVersion, previousInLine } from "./lib.mjs";
import { REPO } from "./publish.mjs";

/** The one folder of migrations (`photos-core`). */
export const MIGRATIONS_DIR = "src-tauri/crates/photos-core/migrations";

/** Which gates run before the build and which need the built candidate. */
export const PHASES = {
  before: ["version", "tag-ref", "changelog", "version-fields", "next-version", "no-migration", "notices", "checks", "env", "rawler", "ffmpeg-source", "source-bundle"],
  after: ["library-check", "setup-sizes"],
};

/** The CI jobs that must be green on the tagged commit (Windows-only releases, decision 4a). */
export const REQUIRED_CHECKS = ["frontend", "scripts", "rust (windows-latest)"];

/**
 * `checks` from GitHub's check runs for the tagged commit. A job counts when any run of it
 * completed successfully: the tag push starts a second CI run that is still going while the
 * release runs, and the run from the push to `main` already answered.
 */
export function checksFromRuns(runs, required = REQUIRED_CHECKS) {
  const green = new Set(runs.filter((r) => r.status === "completed" && r.conclusion === "success").map((r) => r.name));
  const failed = required.filter((name) => !green.has(name));
  return { ok: failed.length === 0, failed };
}

/** Every gate, in the order a person reads them. `why` never carries a secret. */
export function evaluatePreflight(f, { version, ref }) {
  const gates = [];
  const gate = (id, ok, why) => gates.push({ id, ok: !!ok, why: ok ? "" : why });
  let channel = null;
  try {
    channel = parseVersion(version).channel;
    gate("version", true, "");
  } catch (e) {
    gate("version", false, e.message);
  }
  const tag = tagRefGate({ ref: ref ?? f.ref, branches: f.branches }, version);
  gate("tag-ref", tag.ok, `The tag does not qualify: ${tag.why.join("; ")}.`);
  gate("changelog", f.changelogVersions?.includes(version), `docs/CHANGELOG.md has no "## ${version}" entry.`);
  gate("version-fields", f.versionFields === version, `The version fields say ${f.versionFields}. Run node scripts/release/set-version.mjs ${version} and commit.`);
  if (channel) {
    let next = false;
    try {
      next = isNextVersion(f.previousVersion, version);
    } catch {
      next = false;
    }
    gate("next-version", next, `${version} does not follow the last release, ${f.previousVersion}. Every build gets a new number.`);
  }
  // A silent security install never rewrites a schema on a stable line (spec, *Security track*).
  // Previews may migrate (the safety copy still runs first). A stable tag counts wherever it was cut.
  const onStableLine = channel === "stable" || (f.branches ?? []).some((b) => b.startsWith("release/"));
  if (f.security && onStableLine) {
    const unknown = f.migrationsChanged === null || f.migrationsChanged === undefined;
    gate(
      "no-migration",
      f.migrationsChanged === false,
      unknown
        ? `The migrations could not be compared with the previous build, ${f.previousVersion ?? "none"}.`
        : `This security build changes ${MIGRATIONS_DIR} since ${f.previousVersion}; a stable security fix must not migrate.`,
    );
  } else gate("no-migration", true, "");
  gate("notices", f.noticesFresh, "src/assets/licenses/notices.json changed when regenerated. Commit yarn run licenses first.");
  gate("checks", f.checks?.ok, `CI is not green on the tagged commit: ${(f.checks?.failed ?? ["no result"]).join(", ")}.`);
  const set = (f.testOverrides ?? []).filter((n) => TEST_OVERRIDES.includes(n));
  gate("env", set.length === 0, `Test-only settings are set: ${set.join(", ")}.`);
  const sizes = setupSizeGate(f.setupSizes);
  gate("library-check", f.libraryCheck?.ok, `The candidate did not open a synthetic library: ${f.libraryCheck?.why ?? "not run"}.`);
  gate("setup-sizes", sizes.ok, `Lumen Setup is over its size budget: ${sizes.why.join("; ")}.`);
  const deps = f.rawler?.dependents;
  const known = Array.isArray(deps);
  const onlyRaw = known && deps.every((d) => d === "photos-raw");
  const bundled = known && (deps.length === 0 || (f.rawler?.resources ?? []).some((r) => RAW_LIBRARY.test(r.split(/[\/]/).pop())));
  gate(
    "rawler",
    onlyRaw && bundled,
    !known
      ? "cargo tree could not say which packages use rawler."
      : !onlyRaw
        ? `rawler (LGPL-2.1) is linked into ${deps.filter((d) => d !== "photos-raw").join(", ")}; it may only be in the replaceable photos-raw library.`
        : "photos-raw's library is not in tauri.conf.json's bundle.resources.",
  );
  const ff = f.ffmpeg && f.sources?.ffmpeg ? ffmpegGate(f.ffmpeg, f.sources.ffmpeg, f.sourceBundle ?? {}) : { ok: false, why: ["no FFmpeg facts"] };
  gate("ffmpeg-source", ff.ok, `FFmpeg's terms are not met: ${ff.why.join("; ")}.`);
  const others = sourceBundleGate(f.sources?.others ?? [], f.sourceBundle ?? {});
  gate("source-bundle", others.ok, `The source bundle is incomplete: ${others.why.join("; ")}.`);
  return gates;
}

/**
 * The `library-check` fact from what `lumen --check-library` printed for each synthetic library
 * (`<tag>-schema-<n>`): every one must open, from the schema it was made at. Fixtures are made by
 * `src-tauri/tests/library_fixtures.rs`; never a real library.
 */
export function judgeLibraryChecks(answers) {
  if (!answers.length) return { ok: false, why: "no synthetic libraries were made" };
  let last = null;
  for (const { name, stdout } of answers) {
    let out;
    try {
      out = JSON.parse(String(stdout).trim().split(/\r?\n/).pop());
    } catch {
      return { ok: false, why: `${name}: no answer from --check-library` };
    }
    if (!out.ok) return { ok: false, why: `${name}: ${out.why}` };
    const made = Number(/-schema-(\d+)$/.exec(name)?.[1]);
    if (out.from !== made) return { ok: false, why: `${name}: opened from schema ${out.from}, made at ${made}` };
    last = out;
  }
  const first = Number(/-schema-(\d+)$/.exec(answers[0].name)?.[1]);
  return { ok: true, from: first, to: last.to };
}

/**
 * Makes one synthetic library per published schema in a new temp folder (the Rust test
 * `write_fixtures_when_asked`) and opens each with the built Lumen at `exe`. Needs cargo, so it
 * runs in CI after the build; the folder is removed afterwards.
 */
export function runLibraryCheck(exe, root) {
  const out = mkdtempSync(join(tmpdir(), "lumen-fixtures-"));
  try {
    const made = spawnSync("cargo", ["test", "-p", "lumen", "--test", "library_fixtures", "write_fixtures_when_asked"], {
      cwd: join(root, "src-tauri"),
      env: { ...process.env, LUMEN_FIXTURES_OUT: out },
      encoding: "utf8",
    });
    if (made.status !== 0) return { ok: false, why: "the synthetic libraries could not be made" };
    if (!existsSync(exe)) return { ok: false, why: `${exe} was not built` };
    const answers = readdirSync(out).map((name) => ({ name, stdout: spawnSync(exe, ["--check-library", join(out, name)], { encoding: "utf8" }).stdout }));
    return judgeLibraryChecks(answers);
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

// ---- The CLI: facts for the gates -----------------------------------------------------------
// `node scripts/release/preflight.mjs before <version>` runs in the preflight job, before anything
// is built; `... after <version> --sizes <sizes.json> --exe <lumen.exe>` runs in the build job once
// Setup is built. Each prints every gate of its phase and exits 1 if one fails.

const USAGE = "usage: node scripts/release/preflight.mjs before <version> | after <version> --sizes <sizes.json> --exe <lumen.exe>";

export function parseArgs(argv) {
  const [phase, version, ...rest] = argv;
  if (!["before", "after"].includes(phase) || !version) throw new Error(USAGE);
  if (phase === "before") return { phase, version };
  const flag = (name) => (rest.includes(name) ? rest[rest.indexOf(name) + 1] : null);
  const sizes = flag("--sizes");
  const exe = flag("--exe");
  if (!sizes) throw new Error(`after needs --sizes <sizes.json>. ${USAGE}`);
  if (!exe) throw new Error(`after needs --exe <lumen.exe>. ${USAGE}`);
  return { phase, version, sizes, exe };
}

/** The branches (no remote prefix) from `git branch -r --contains HEAD`. */
export function branchesFrom(output) {
  return String(output)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.includes("->"))
    .map((l) => l.replace(/^[^/]+\//, ""));
}

/** The published build `currentTag` has to follow, within its line (`previousInLine`); `null` before the first. */
export function previousVersionFrom(releases, currentTag) {
  const published = releases.filter((r) => !r.isDraft && r.tagName !== currentTag).map((r) => r.tagName.replace(/^v/, ""));
  return previousInLine(published, currentTag.replace(/^v/, ""));
}

/** `migrationsChanged` from `git diff --name-only <previous tag> HEAD -- <migrations>`: no output is unchanged, a git failure is unknown (`null`). */
export function migrationsChangedFrom({ ok, out }) {
  return ok ? out.trim() !== "" : null;
}

/** `{ file: sha256 }` from a release's assets (`gh release view --json assets`); no digest, no entry. */
export function bundleFromAssets(assets) {
  return Object.fromEntries(assets.filter((a) => /^sha256:[0-9a-f]+$/i.test(a.digest ?? "")).map((a) => [a.name, a.digest.slice(7).toLowerCase()]));
}

const RAW_NAMES = { win32: "photos_raw.dll", darwin: "libphotos_raw.dylib", linux: "libphotos_raw.so" };

/**
 * File names a `bundle.resources` list stands for. `binaries/raw/*` is the folder `build.rs` fills
 * with the photos-raw library, which is not built yet when the preflight runs, so an empty folder
 * stands for the library `build.rs` is about to put there.
 */
export function resourceNames(keys, listDir, platform = process.platform) {
  return keys.flatMap((key) => {
    if (key.endsWith("/*")) {
      const dir = key.slice(0, -2);
      const files = listDir(dir);
      if (files.length) return files;
      if (dir === "binaries/raw") return [RAW_NAMES[platform] ?? RAW_NAMES.linux];
    }
    return [key.split("/").filter(Boolean).pop()];
  });
}

/** The gates of one phase, in the order `evaluatePreflight` lists them. */
export function runGates(facts, { version, ref, phase }) {
  const ids = PHASES[phase];
  return evaluatePreflight(facts, { version, ref }).filter((g) => ids.includes(g.id));
}

export function formatReport(gates) {
  return gates.map((g) => `${g.ok ? "ok  " : "FAIL"}  ${g.id}${g.ok ? "" : `: ${g.why}`}`).join("\n");
}

function run(cmd, args, cwd) {
  const r = spawnSync(cmd, args, { cwd, encoding: "utf8", shell: process.platform === "win32" && cmd === "yarn" });
  return { ok: r.status === 0, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
}

const sha256File = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

/** The facts the `before` gates need. Reads the checkout, `cargo`, `git` and `gh`; `yarn run licenses` rewrites notices.json, which is then compared with git. */
export function gatherBefore(root, { version, repo, sha, env = process.env }) {
  const conf = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
  const sidecar = join(root, "src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe");
  const gh = (args) => run("gh", args, root);
  const json = (r, fallback) => {
    try {
      return JSON.parse(r.out);
    } catch {
      return fallback;
    }
  };
  const assets = json(gh(["release", "view", env.FFMPEG_SOURCES_TAG, "--repo", repo, "--json", "assets"]), { assets: [] }).assets;
  const releases = json(gh(["release", "list", "--repo", repo, "--limit", "200", "--json", "tagName,isDraft"]), []);
  const pages = json(gh(["api", "--paginate", "--slurp", `repos/${repo}/commits/${sha}/check-runs?per_page=100`]), []);
  const notices = run("yarn", ["-s", "run", "licenses"], root);
  const tree = run("cargo", ["tree", "-i", "rawler", "-e", "normal", "--depth", "1", "--prefix", "none"], join(root, "src-tauri"));
  const changelog = readFileSync(join(root, "docs/CHANGELOG.md"), "utf8");
  const previousVersion = previousVersionFrom(releases, `v${version}`);
  const security = isSecurityRelease(changelog, version);
  const resources = conf.bundle?.resources ?? [];
  const keys = Array.isArray(resources) ? resources : Object.keys(resources);
  const listDir = (dir) => {
    const p = join(root, "src-tauri", dir);
    return existsSync(p) ? readdirSync(p) : [];
  };
  return {
    ref: env.GITHUB_REF ?? `refs/tags/v${version}`,
    branches: branchesFrom(run("git", ["branch", "-r", "--contains", "HEAD"], root).out),
    changelogVersions: [...changelog.matchAll(/^## (\S+)/gm)].map((m) => m[1]),
    versionFields: JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version,
    previousVersion,
    security,
    migrationsChanged: security && previousVersion ? migrationsChangedFrom(run("git", ["diff", "--name-only", `v${previousVersion}`, "HEAD", "--", MIGRATIONS_DIR], root)) : null,
    noticesFresh: notices.ok && run("git", ["status", "--porcelain", "--", "src/assets/licenses/notices.json"], root).out.trim() === "",
    checks: checksFromRuns(pages.flatMap((p) => p.check_runs ?? [])),
    testOverrides: TEST_OVERRIDES.filter((k) => env[k]),
    rawler: { dependents: rawlerDependents(tree.out), resources: resourceNames(keys, listDir) },
    ffmpeg: existsSync(sidecar) ? { versionOutput: run(sidecar, ["-hide_banner", "-version"], root).out, sidecarSha256: sha256File(sidecar) } : { versionOutput: null, sidecarSha256: null },
    sources: JSON.parse(readFileSync(join(root, "scripts/release/sources.json"), "utf8")),
    sourceBundle: bundleFromAssets(assets),
  };
}

function main(argv) {
  const args = parseArgs(argv);
  const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
  let facts;
  if (args.phase === "before") {
    const sha = process.env.GITHUB_SHA ?? run("git", ["rev-parse", "HEAD"], root).out.trim();
    facts = gatherBefore(root, { version: args.version, repo: process.env.GITHUB_REPOSITORY ?? REPO, sha });
  } else {
    facts = { setupSizes: JSON.parse(readFileSync(args.sizes, "utf8")), libraryCheck: runLibraryCheck(args.exe, root) };
  }
  const gates = runGates(facts, { version: args.version, ref: facts.ref, phase: args.phase });
  console.log(formatReport(gates));
  return gates.every((g) => g.ok) ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (e) {
    console.error(e.message);
    process.exit(2);
  }
}
