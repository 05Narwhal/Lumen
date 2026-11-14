// `node scripts/oss/export-public.mjs <empty-dest-dir>` — makes the scrubbed one-commit snapshot of
// `HEAD` that becomes the public repository (docs/superpowers/plans/2026-11-14-open-source.md,
// Task 10). It never pushes and never creates anything on GitHub.
//
//   1. refuses a destination that is not empty, is inside the repo, or is inside (or holds) any
//      library listed in the app's libraries.json (that file is only read, for its paths);
//   2. `git archive HEAD` into a staging folder in the temp directory;
//   3. drops the paths in public-exclude.txt and scans every other file with scrub.mjs's rules
//      (plus the git-ignored local rules when present). A finding stops the export: nothing is
//      written to <dest>, the staging folder is left for inspection and its path is printed;
//   4. copies the clean files to <dest>, `git init -b main`, and commits as the noreply identity
//      with the DCO sign-off;
//   5. prints the manual steps. Julian does those.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadRules, scanFile, selectFiles } from "./scrub.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(here, "..", "..");

export const AUTHOR_NAME = "05Narwhal";
export const AUTHOR_EMAIL = "05Narwhal@users.noreply.github.com";

/** `inner` is `outer` or lies under it (case-insensitive: Windows and macOS file systems are). */
function within(inner, outer) {
  const rel = relative(resolve(outer).toLowerCase(), resolve(inner).toLowerCase());
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
}

/** An error message when `dest` must not be exported to, else null. */
export function checkDest(dest, repoRoot, libraryPaths) {
  if (within(dest, repoRoot)) return `${dest} is inside the repo (or is the repo); pick a folder outside it.`;
  for (const lib of libraryPaths) {
    if (within(dest, lib) || within(lib, dest)) return `${dest} is inside, or holds, the library ${lib}; never export near a library.`;
  }
  if (existsSync(dest)) {
    if (!statSync(dest).isDirectory()) return `${dest} exists and is not a folder.`;
    if (readdirSync(dest).length > 0) return `${dest} is not empty; pick a new empty folder.`;
  }
  return null;
}

/** The library folders the app has registered, read-only. Missing or unreadable means none known. */
export function registeredLibraries(env = process.env) {
  const bases = [env.APPDATA && join(env.APPDATA, "JL-Studios", "Lumen"), join(homedir(), ".config", "JL-Studios", "Lumen"), join(homedir(), "Library", "Application Support", "JL-Studios", "Lumen")].filter(Boolean);
  const paths = [];
  for (const base of bases) {
    try {
      const reg = JSON.parse(readFileSync(join(base, "libraries.json"), "utf8"));
      for (const l of reg.libraries ?? []) if (l.path) paths.push(String(l.path));
    } catch {
      // not there, or not readable: nothing to learn from this location
    }
  }
  return paths;
}

function walk(dir, base = dir, out = []) {
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, name.name);
    if (name.isDirectory()) walk(full, base, out);
    else out.push(relative(base, full).replace(/\\/g, "/"));
  }
  return out;
}

const git = (args, opts = {}) => execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...opts });

export const MANUAL_STEPS = (dest) => `
Snapshot ready: ${dest}
(one commit, author ${AUTHOR_NAME} <${AUTHOR_EMAIL}>). Nothing was pushed or created on GitHub.

Your manual steps:
  1. Rename 05Narwhal/lumen to 05Narwhal/lumen-dev-archive and keep it private.
  2. Create a public 05Narwhal/lumen and push the snapshot (cd into the folder above, then
     git remote add origin https://github.com/05Narwhal/lumen.git and git push -u origin main).
  3. Make 05Narwhal/lumen-models public.
  4. Turn on Discussions, private vulnerability reporting and branch protection on main (CI and DCO required).
  5. Add the secrets: TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD, LUMEN_PLUGIN_PUBKEY, LUMEN_DISCORD_CLIENT_ID.
  6. Point the dev checkout's origin at the public repo and keep the archive as a second remote.
  7. Apply to SignPath Foundation.
  8. Set up GitHub Sponsors.
`;

export function exportPublic(dest, { repoRoot = REPO_ROOT, libraries = registeredLibraries(), log = console.log } = {}) {
  const problem = checkDest(dest, repoRoot, libraries);
  if (problem) throw new Error(problem);

  const version = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8")).version;
  const stage = mkdtempSync(join(tmpdir(), "lumen-export-stage-"));
  const tarball = join(stage, "snapshot.tar");
  const tree = join(stage, "tree");
  mkdirSync(tree);
  git(["archive", "--format=tar", "-o", tarball, "HEAD"], { cwd: repoRoot });
  // Relative paths: GNU tar reads `C:` in an absolute path as a remote host name.
  execFileSync("tar", ["-xf", "snapshot.tar", "-C", "tree"], { cwd: stage, stdio: "ignore" });
  rmSync(tarball, { force: true });

  const excludes = readFileSync(join(here, "public-exclude.txt"), "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
  const all = walk(tree);
  const kept = selectFiles(all, excludes);
  const rules = loadRules(join(here, "scrub-rules.json"), join(here, "scrub-rules.local.json"));

  const findings = [];
  for (const f of kept) findings.push(...scanFile(f, readFileSync(join(tree, f)), rules));
  if (findings.length > 0) {
    const lines = findings.map((x) => `${x.path}${x.line ? ":" + x.line : ""} ${x.rule}`);
    const err = new Error(`${findings.length} scrub findings; nothing was written to ${dest}. The staging copy is left at ${tree}.\n${lines.join("\n")}`);
    err.findings = findings;
    throw err;
  }

  const keep = new Set(kept);
  for (const f of all) if (!keep.has(f)) rmSync(join(tree, f), { force: true });
  cpSync(tree, dest, { recursive: true });
  rmSync(stage, { recursive: true, force: true });
  log(`${kept.length} files copied, ${all.length - kept.length} excluded.`);

  const identity = { GIT_AUTHOR_NAME: AUTHOR_NAME, GIT_AUTHOR_EMAIL: AUTHOR_EMAIL, GIT_COMMITTER_NAME: AUTHOR_NAME, GIT_COMMITTER_EMAIL: AUTHOR_EMAIL };
  const env = { ...process.env, ...identity };
  git(["init", "-b", "main"], { cwd: dest, env });
  git(["add", "-A"], { cwd: dest, env });
  const msgFile = join(dest, "..", `${dest.split(/[\\/]/).filter(Boolean).pop()}.commit-msg.txt`);
  writeFileSync(msgFile, `Lumen ${version} — first public release\n\nSigned-off-by: ${AUTHOR_NAME} <${AUTHOR_EMAIL}>\n`, "utf8");
  try {
    // The snapshot is signed by nobody: a personal signing key would carry a personal email.
    git(["-c", "commit.gpgsign=false", "commit", "-F", msgFile], { cwd: dest, env });
  } finally {
    rmSync(msgFile, { force: true });
  }
  const author = git(["log", "-1", "--format=%an <%ae> / %cn <%ce>"], { cwd: dest }).trim();
  const expected = `${AUTHOR_NAME} <${AUTHOR_EMAIL}> / ${AUTHOR_NAME} <${AUTHOR_EMAIL}>`;
  if (author !== expected) throw new Error(`the snapshot's commit is by ${author}, expected ${expected}`);
  const count = git(["rev-list", "--count", "HEAD"], { cwd: dest }).trim();
  if (count !== "1") throw new Error(`the snapshot has ${count} commits, expected 1`);
  return { version, files: kept.length };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dest = process.argv[2];
  if (!dest) {
    console.error("usage: node scripts/oss/export-public.mjs <new-empty-folder>");
    process.exit(2);
  }
  try {
    const dir = resolve(dest);
    exportPublic(dir);
    console.log(MANUAL_STEPS(dir));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
