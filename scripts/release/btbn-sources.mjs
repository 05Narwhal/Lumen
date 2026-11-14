// Round 17 / FFmpeg route B: the source list for a BtbN FFmpeg build, read from BtbN's own
// `scripts.d` at the commit that built it (BtbN/FFmpeg-Builds pins every library by commit there).
// This file is the offline half: parse the scripts, find which of them went into the binary
// (its `--enable-*` flags plus their `ffbuild_depends`, transitively) and say where each one's
// exact source is. Downloading and SHA-256 (`--fetch`) is `btbn-fetch.mjs`'s job.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";

const body = (text, fn) => {
  const m = new RegExp(`^${fn}\\(\\) \\{\\n([\\s\\S]*?)^\\}`, "m").exec(text);
  return m ? m[1] : "";
};
const assign = (text, key) => new RegExp(`^${key}="([^"]*)"`, "m").exec(text)?.[1] ?? null;

/** The folders a script's `git submodule update` names (none: every submodule). */
function submodulePaths(text) {
  const line = /git submodule update([^"\n]*)"/.exec(text)?.[1] ?? "";
  return line
    .replace(/--(?:init|recursive|recommend-shallow)\b/g, "")
    .replace(/--(?:depth|filter)(?:=|\s+)\S+/g, "")
    .split(/\s+/)
    .filter(Boolean);
}

/** One `scripts.d/*.sh`: its pinned sources, the stages it needs and the flags it turns on. */
export function parseScript(raw, path) {
  const text = raw.replaceAll("\r\n", "\n");
  const depends = [...body(text, "ffbuild_depends").matchAll(/^\s*echo\s+(\S+)/gm)].map((m) => m[1]);
  const flags = [...body(text, "ffbuild_configure").matchAll(/--enable-([a-z0-9-]+)/g)].map((m) => m[1]);
  const sources = [];
  for (const n of ["", "2", "3"]) {
    const repo = assign(text, `SCRIPT_REPO${n}`);
    const commit = assign(text, `SCRIPT_COMMIT${n}`);
    if (repo && commit) sources.push({ repo, commit, branch: assign(text, `SCRIPT_BRANCH${n}`), n: n || "1" });
  }
  return {
    path,
    name: basename(path).replace(/\.sh$/, "").replace(/^\d\d-/, ""),
    skip: assign(text, "SCRIPT_SKIP") === "1",
    svnRev: assign(text, "SCRIPT_REV"),
    repo: assign(text, "SCRIPT_REPO"),
    sources,
    submodules: /git submodule update/.test(text),
    submodulePaths: submodulePaths(text),
    depends,
    flags,
  };
}

/** Every stage by name; a directory is one stage made of its scripts. */
export function loadStages(dir) {
  const stages = new Map();
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    const name = entry.replace(/\.sh$/, "").replace(/^\d\d-/, "");
    const files = statSync(full).isDirectory()
      ? readdirSync(full).filter((f) => f.endsWith(".sh")).sort().map((f) => join(full, f))
      : [full];
    stages.set(name, files.map((f) => parseScript(readFileSync(f, "utf8"), f.slice(dir.length - "scripts.d".length).replace(/\\/g, "/"))));
  }
  return stages;
}

/**
 * The stages that went into a binary configured with `enabled` flags: the ones whose
 * `ffbuild_configure` turns a flag on, and everything they depend on. `options` are flags that
 * are not a library (gpl, pthreads…); `system` are Windows interfaces with no source to give.
 * Flags no script claims come back in `unclaimed`, so nothing is silently left out.
 */
export function stagesFor(stages, enabled, { options = [], system = [] } = {}) {
  const wanted = new Set();
  const unclaimed = [];
  const claimedBy = (flag) => [...stages].filter(([, ss]) => ss.some((s) => s.flags.includes(flag))).map(([n]) => n);
  for (const flag of enabled) {
    if (options.includes(flag) || system.includes(flag)) continue;
    const owners = claimedBy(flag);
    if (!owners.length) unclaimed.push(flag);
    owners.forEach((o) => wanted.add(o));
  }
  const queue = [...wanted];
  while (queue.length) {
    const name = queue.pop();
    for (const s of stages.get(name) ?? []) {
      for (const d of s.depends) {
        if (stages.has(d) && !wanted.has(d)) {
          wanted.add(d);
          queue.push(d);
        }
      }
    }
  }
  return { stages: [...wanted].sort(), unclaimed };
}

const GITLABS = /^(gitlab\.[a-z.]+|code\.videolan\.org|invent\.kde\.org|salsa\.debian\.org)$/;

/** Where a commit's source archive is, for the hosts BtbN's scripts use; null when unknown. */
export function archiveUrl(repoUrl, ref) {
  const u = new URL(repoUrl.replace(/\.git$/, ""));
  const path = u.pathname.replace(/^\/|\/$/g, "");
  const base = path.split("/").pop();
  const file = `${base}-${ref.replace(/\//g, "_")}.tar.gz`;
  if (u.host === "github.com") return { url: `https://github.com/${path}/archive/${ref}.tar.gz`, file };
  if (GITLABS.test(u.host)) return { url: `https://${u.host}/${path}/-/archive/${ref}/${file}`, file };
  if (u.host.endsWith(".googlesource.com")) return { url: `https://${u.host}/${path}/+archive/${ref}.tar.gz`, file };
  if (u.host === "git.code.sf.net") {
    const [, project, repo] = path.match(/^p\/([^/]+)\/([^/]+)$/) ?? [];
    if (project) return { url: `https://sourceforge.net/p/${project}/${repo}/ci/${ref}/tarball`, file };
  }
  if (u.host === "git.ffmpeg.org") return { url: `https://git.ffmpeg.org/gitweb/${path}.git/snapshot/${ref}.tar.gz`, file };
  if (u.host === "git.savannah.gnu.org") return { url: `https://git.savannah.gnu.org/cgit/${path.replace(/^git\//, "")}.git/snapshot/${file}`, file };
  return null;
}

/** Extra sources a script lists that this build never uses (older SDKs for older FFmpegs). */
const UNUSED_EXTRAS = { ffnvcodec: ["2", "3"] };

/**
 * One component per library flag (the gate looks them up by flag) and one flag-less one per
 * library nothing turns on by itself. Entries with no URL say why in `note`, so the gate refuses
 * them visibly until someone fills them.
 */
export function componentsFor(stages, wantedNames, { scriptsCommit }) {
  const out = [];
  for (const name of wantedNames) {
    for (const s of stages.get(name)) {
      if (s.skip && !s.sources.length) continue;
      const used = s.sources.filter((x) => !(UNUSED_EXTRAS[name] ?? []).includes(x.n));
      const rows = used.length ? used : [null];
      for (const src of rows) {
        const label = src && src.n !== "1" ? `${s.name} (${basename(src.repo, ".git")})` : s.name;
        const base = { name: label, script: s.path };
        let located = { url: null, sha256: null, file: null };
        let note;
        if (!src) note = s.svnRev ? `subversion revision ${s.svnRev} of ${s.repo}: no archive to link` : "no source recorded in the script";
        else {
          const a = archiveUrl(src.repo, src.commit);
          if (a) located = { ...a, sha256: null };
          else note = `${src.repo} @ ${src.commit}: no archive URL known for this host`;
        }
        const flags = s.flags.length ? s.flags : [null];
        for (const flag of flags) {
          out.push({ ...base, flag, ...located, exact: src?.commit ?? s.svnRev ?? null, ...(note ? { note } : {}), ...(s.submodules && src?.n === "1" ? { submodules: true, submodulePaths: s.submodulePaths, repo: src.repo } : {}) });
        }
      }
    }
  }
  return out;
}
