// Round 17 / FFmpeg route B: turns btbn-plan's list into `sources.json` → `ffmpeg`.
//   node scripts/release/btbn-fetch.mjs <FFmpeg-Builds checkout at the scripts commit> <ffmpeg -version file> [--sidecar-sha <hex>] [--write]
// For every library the binary needs it downloads the pinned archive (five tries each), records the
// SHA-256 and prints what is still unresolved. Without --write the result goes to the scratchpad-style
// `dist-release/ffmpeg/sources.draft.json`; with --write it replaces `ffmpeg` in sources.json and
// drops the `others` rows that only described gyan's build. Downloaded files are kept in
// `dist-release/ffmpeg/source-cache/`, so a rerun only fetches what is missing.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";
import { archiveUrl } from "./btbn-sources.mjs";
import { plan } from "./btbn-plan.mjs";
import { download } from "./source-bundle.mjs";

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

/** `.gitmodules` text → [{ path, url }]. */
export function parseGitmodules(text) {
  const out = [];
  for (const block of text.split(/^\[submodule [^\]]*\]\s*$/m).slice(1)) {
    const path = /^\s*path\s*=\s*(.+?)\s*$/m.exec(block)?.[1];
    const url = /^\s*url\s*=\s*(.+?)\s*$/m.exec(block)?.[1];
    if (path && url) out.push({ path, url });
  }
  return out;
}

/** `git ls-tree -r` output → the gitlinks (submodule commits): [{ path, commit }]. */
export function parseGitlinks(lsTree) {
  return [...lsTree.matchAll(/^160000 commit ([0-9a-f]{40})\t(.+)$/gm)].map((m) => ({ commit: m[1], path: m[2] }));
}

/** Cargo.lock → the crates.io packages with their checksums; throws on a source it cannot link. */
export function parseCargoLock(text) {
  const out = [];
  for (const block of text.split(/^\[\[package\]\]\s*$/m).slice(1)) {
    const name = /^name = "([^"]+)"/m.exec(block)?.[1];
    const version = /^version = "([^"]+)"/m.exec(block)?.[1];
    const source = /^source = "([^"]+)"/m.exec(block)?.[1];
    if (!source) continue; // a member of the workspace: it is in the repository archive
    if (!source.startsWith("registry+https://github.com/rust-lang/crates.io-index")) throw new Error(`${name} ${version}: source ${source} is not crates.io`);
    const checksum = /^checksum = "([0-9a-f]{64})"/m.exec(block)?.[1];
    if (!checksum) throw new Error(`${name} ${version}: no checksum in Cargo.lock`);
    out.push({ name, version, checksum });
  }
  return out;
}

export const crateRow = (c, owner) => ({
  name: `${c.name} ${c.version} (crate, built into ${owner})`,
  flag: null,
  url: `https://static.crates.io/crates/${c.name}/${c.name}-${c.version}.crate`,
  sha256: c.checksum,
  file: `crate-${c.name}-${c.version}.crate`,
  exact: `${c.name} ${c.version}`,
});

const git = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8", maxBuffer: 64 << 20 });

/** The submodules a repository at `ref` pins (recursively), for the folders the script initialises. */
export function resolveSubmodules(repoUrl, ref, only = [], seen = new Set()) {
  const key = `${repoUrl}@${ref}`;
  if (seen.has(key)) return [];
  seen.add(key);
  const dir = mkdtempSync(join(tmpdir(), "btbn-sub-"));
  try {
    git(dir, "init", "-q");
    git(dir, "fetch", "-q", "--depth=1", "--filter=blob:none", repoUrl, ref);
    const links = parseGitlinks(git(dir, "ls-tree", "-r", "FETCH_HEAD"));
    if (!links.length) return [];
    const urls = new Map(parseGitmodules(git(dir, "show", "FETCH_HEAD:.gitmodules")).map((m) => [m.path, m.url]));
    const out = [];
    for (const l of links) {
      if (only.length && !only.some((p) => l.path === p || l.path.startsWith(`${p}/`))) continue;
      const rel = urls.get(l.path);
      const url = rel && new URL(rel, repoUrl).href;
      if (!rel) throw new Error(`${repoUrl}: submodule ${l.path} has no url in .gitmodules`);
      out.push({ path: l.path, url, commit: l.commit });
      for (const deeper of resolveSubmodules(url, l.commit, [], seen)) out.push({ ...deeper, path: `${l.path}/${deeper.path}` });
    }
    return out;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const RUST = { rav1e: "rav1e", librsvg: "librsvg" };
const VENDORED = {
  libmp3lame: { local: "scripts/release/vendored-source/lame-r7113.tar.gz", note: "Subversion revision 7113 of https://svn.code.sf.net/p/lame/svn/trunk/lame, which LAME publishes no archive of: exported over Subversion's HTTP interface by scripts/release/svn-export.mjs (stored file content, no svn:keywords or svn:eol-style translation) and kept in this repository." },
  libxvid: { local: "scripts/release/vendored-source/xvidcore-r2204.tar.gz", note: "Subversion revision 2204 of https://svn.xvid.org/trunk/xvidcore, which Xvid publishes no archive of: exported over Subversion's HTTP interface by scripts/release/svn-export.mjs (stored file content, no svn:keywords or svn:eol-style translation) and kept in this repository." },
};

async function getJson(url) {
  const res = await fetch(url, { headers: { "User-Agent": "lumen-release-script" } });
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  return res.json();
}

/** Everything the gate needs, still without hashes for archives (the caller downloads those). */
export async function buildRows(checkout, versionOutput, root) {
  const scriptsCommit = git(checkout, "rev-parse", "HEAD").trim();
  const p = plan(checkout, versionOutput);
  const short = /-g([0-9a-f]+)-/.exec(p.version)?.[1];
  const date = /-(\d{8})$/.exec(p.version)?.[1];
  if (!short) throw new Error(`cannot read FFmpeg's commit out of ${p.version}`);
  const full = (await getJson(`https://api.github.com/repos/FFmpeg/FFmpeg/commits/${short}`)).sha;
  const rows = [
    { name: "FFmpeg", flag: null, ...archiveUrl("https://github.com/FFmpeg/FFmpeg.git", full), exact: full },
    { name: "BtbN/FFmpeg-Builds (build scripts and the patches it applies)", flag: null, ...archiveUrl("https://github.com/BtbN/FFmpeg-Builds.git", scriptsCommit), exact: scriptsCommit },
  ];
  for (const c of p.components) {
    const v = VENDORED[c.flag];
    if (v) {
      rows.push({ name: c.name, script: c.script, flag: c.flag, local: v.local, file: v.local.split("/").pop(), exact: c.exact, note: v.note });
      continue;
    }
    const { submodules, submodulePaths, repo, ...row } = c;
    rows.push(row);
    if (submodules && row.url) {
      for (const s of resolveSubmodules(repo, row.exact, submodulePaths)) {
        const a = archiveUrl(s.url, s.commit);
        rows.push({ name: `${row.name}: submodule ${s.path}`, flag: null, ...(a ?? { url: null, file: null, sha256: null, note: `${s.url} @ ${s.commit}: no archive URL known for this host` }), exact: s.commit });
      }
    }
  }
  const rustSeen = new Set();
  for (const c of p.components) {
    if (!(c.name in RUST) || rustSeen.has(c.name)) continue;
    rustSeen.add(c.name);
    const repo = /github\.com\/([^/]+\/[^/]+?)\/archive\//.exec(c.url)?.[1];
    const res = await fetch(`https://raw.githubusercontent.com/${repo}/${c.exact}/Cargo.lock`);
    if (!res.ok) throw new Error(`${repo}@${c.exact}: Cargo.lock answered ${res.status}`);
    for (const k of parseCargoLock(await res.text())) rows.push(crateRow(k, c.name));
  }
  return { rows, version: p.version, date, scriptsCommit, enabled: p.enabled };
}

/** Download each distinct archive once (four at a time), record its SHA-256; crates already carry theirs. */
export async function hashRows(rows, cacheDir, { fetchFn = fetch, retryDelayMs = 3000, say = console.log, root = resolve(".") } = {}) {
  const gitDir = join(cacheDir, "git");
  mkdirSync(gitDir, { recursive: true });
  const todo = [...new Map(rows.filter((r) => r.url && !r.sha256).map((r) => [r.file, r])).values()];
  const hashes = new Map();
  const madeByGit = new Set();
  const failed = new Map();
  let next = 0;
  async function worker() {
    while (next < todo.length) {
      const r = todo[next++];
      const target = join(cacheDir, r.file);
      const gitTarget = join(gitDir, r.file);
      try {
        if (existsSync(gitTarget)) {
          madeByGit.add(r.file);
          hashes.set(r.file, sha256(readFileSync(gitTarget)));
          continue;
        }
        if (existsSync(target)) {
          hashes.set(r.file, sha256(readFileSync(target)));
          continue;
        }
        let got;
        try {
          got = await download(r.url, { fetchFn, retryDelayMs });
        } catch (e) {
          // GitHub resets the 400 MB AMF archive every time: the same commit fetched by git is retried by git itself.
          if (!gitRepoOf(r.url)) throw e;
          got = { status: e?.message ?? "connection failed" };
        }
        if (got.status) {
          // The host refuses a plain download (GitLab and Savannah do): take the same commit with git.
          const repo = gitRepoOf(r.url);
          if (!repo) throw new Error(`answered ${got.status}`);
          gitArchive(repo, r.exact, r.file.replace(/\.tar\.gz$/, ""), gitTarget);
          madeByGit.add(r.file);
          hashes.set(r.file, sha256(readFileSync(gitTarget)));
          say(`  ${r.file} (git archive of ${repo})`);
          continue;
        }
        writeFileSync(`${target}.part`, got.body);
        renameSync(`${target}.part`, target);
        hashes.set(r.file, sha256(got.body));
        say(`  ${r.file} ${(got.body.length / 1e6).toFixed(1)} MB`);
      } catch (e) {
        failed.set(r.file, `${r.name}: ${r.url} ${e?.message ?? e}`);
      }
    }
  }
  await Promise.all([worker(), worker(), worker(), worker()]);
  for (const r of rows) {
    if (r.url && !r.sha256 && hashes.has(r.file)) r.sha256 = hashes.get(r.file);
    if (r.url && madeByGit.has(r.file)) {
      const from = r.url;
      r.local = relative(root, join(gitDir, r.file)).replace(/\\/g, "/");
      delete r.url;
      r.note = `${new URL(from).host} refuses a plain download of this archive: made with \`git archive\` from ${gitRepoOf(from)} at ${r.exact} and kept in the release's source folder (not in this repository). Original archive URL: ${from}`;
    }
    if (r.local && !r.sha256) r.sha256 = sha256(readFileSync(resolve(root, r.local)));
  }
  return [...failed.values()];
}

/** The git URL behind a GitLab or Savannah archive URL; null for any other host. */
export function gitRepoOf(url) {
  const lab = /^(https:\/\/[^/]+\/.+?)\/-\/archive\//.exec(url)?.[1];
  if (lab) return `${lab}.git`;
  const sav = /^https:\/\/git\.savannah\.gnu\.org\/cgit\/(.+?)\/snapshot\//.exec(url)?.[1];
  if (sav) return `https://git.savannah.gnu.org/git/${sav}`;
  const hub = /^(https:\/\/github\.com\/[^/]+\/[^/]+)\/archive\//.exec(url)?.[1];
  return hub ? `${hub}.git` : null;
}

/** One commit's tree as a tar.gz with a top folder `prefix/` (what the hosts' own archives look like). */
export function gitArchive(repo, ref, prefix, out) {
  const dir = mkdtempSync(join(tmpdir(), "btbn-git-"));
  try {
    git(dir, "init", "-q");
    // Nothing is checked out here, so Windows' file-name rules (a colon in a sample path in pulseaudio) do not apply.
    const fetchArgs = ["-c", "core.protectNTFS=false", "-c", "fetch.fsckObjects=false", "-c", "transfer.fsckObjects=false", "fetch", "-q", "--depth=1", repo, ref];
    for (let attempt = 1; ; attempt++) {
      try {
        git(dir, ...fetchArgs);
        break;
      } catch (e) {
        if (attempt >= 6) throw e; // GitHub resets big transfers (AMF) again and again; git starts over each time
      }
    }
    git(dir, "-c", "core.protectNTFS=false", "archive", "--format=tar.gz", `--prefix=${prefix}/`, "-o", `${out}.part`, "FETCH_HEAD");
    renameSync(`${out}.part`, out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1]?.endsWith("btbn-fetch.mjs")) {
  const args = process.argv.slice(2);
  const [checkout, versionFile] = args;
  const sidecarSha = args.includes("--sidecar-sha") ? args[args.indexOf("--sidecar-sha") + 1] : null;
  const root = resolve(".");
  const dir = join(root, "dist-release/ffmpeg");
  const { rows, version, scriptsCommit } = await buildRows(checkout, readFileSync(versionFile, "utf8"), root);
  console.log(`${rows.length} rows (${rows.filter((r) => r.url && !r.sha256).length} to download)`);
  const failed = await hashRows(rows, join(dir, "source-cache"), { root });
  const open = rows.filter((r) => !r.sha256 || !r.file || (!r.url && !r.local));
  console.log(`failed downloads (${failed.length}):\n${failed.map((f) => `  ${f}`).join("\n")}`);
  console.log(`rows still without a source (${open.length}):\n${open.map((r) => `  ${r.name}: ${r.note ?? "no hash"}`).join("\n")}`);
  const ffmpeg = {
    version,
    sidecar: "src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe",
    sidecarSha256: sidecarSha,
    build: `BtbN/FFmpeg-Builds ${version} (win64 gpl, scripts at ${scriptsCommit})`,
    options: ["gpl", "version3", "pthreads", "schannel"],
    components: rows,
  };
  if (args.includes("--write")) {
    const file = join(root, "scripts/release/sources.json");
    const s = JSON.parse(readFileSync(file, "utf8"));
    s.ffmpeg = ffmpeg;
    s.others = s.others.filter((o) => !/\(linked into FFmpeg\)$/.test(o.name));
    writeFileSync(file, `${JSON.stringify(s, null, 2)}\n`);
    console.log("sources.json written");
  } else {
    writeFileSync(join(dir, "sources.draft.json"), JSON.stringify(ffmpeg, null, 1));
    console.log("draft written to dist-release/ffmpeg/sources.draft.json");
  }
}
