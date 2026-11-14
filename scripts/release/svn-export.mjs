// Round 17 / FFmpeg route B: two libraries in BtbN's build (LAME, Xvid) are pinned to Subversion
// revisions, and neither project publishes an archive of that revision. This reads a revision over
// Subversion's HTTP (WebDAV) interface — no `svn` program needed — and writes it as a tar.gz that
// is the same bytes every time, so its SHA-256 can be recorded and the file shipped with the
// release's source.
//   node scripts/release/svn-export.mjs <repo root URL> <path> <revision> <out.tar.gz> [--anonymous] [--prefix name]
// e.g. https://svn.xvid.org  trunk/xvidcore  2204 xvid-r2204.tar.gz --anonymous
// Files are the stored content (no svn:keywords or svn:eol-style translation), which is what
// `svn export` does too except for those two properties.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const decode = (s) => decodeURIComponent(s.replace(/&amp;/g, "&"));

/** The entries of a PROPFIND Depth:1 answer: href (decoded) and whether it is a folder. */
export function parseMultistatus(xml) {
  const out = [];
  for (const m of xml.matchAll(/<(?:[A-Za-z]+:)?response\b[^>]*>([\s\S]*?)<\/(?:[A-Za-z]+:)?response>/g)) {
    const href = /<(?:[A-Za-z]+:)?href>([^<]*)<\//.exec(m[1])?.[1];
    if (!href) continue;
    out.push({ href: decode(href), dir: /<(?:[A-Za-z]+:)?collection\b/.test(m[1]) });
  }
  return out;
}

export async function listAndFetch(root, path, rev, { fetchFn = fetch, anonymous = false, onFile }) {
  const headers = anonymous ? { Authorization: `Basic ${Buffer.from("anonymous:").toString("base64")}` } : {};
  const base = `${root.replace(/\/$/, "")}/!svn/bc/${rev}/${path.replace(/^\/|\/$/g, "")}`;
  const hostPath = new URL(base).pathname.replace(/\/$/, "");
  async function walk(url, rel) {
    const res = await fetchFn(url + "/", { method: "PROPFIND", headers: { ...headers, Depth: "1" } });
    if (!res.ok && res.status !== 207) throw new Error(`${url}: ${res.status}`);
    for (const e of parseMultistatus(await res.text())) {
      const sub = e.href.replace(/\/$/, "");
      if (sub === new URL(url).pathname.replace(/\/$/, "")) continue;
      const name = sub.slice(sub.lastIndexOf("/") + 1);
      const childUrl = `${new URL(url).origin}${sub}`;
      if (e.dir) await walk(childUrl, `${rel}${name}/`);
      else {
        const f = await fetchFn(childUrl, { headers });
        if (!f.ok) throw new Error(`${childUrl}: ${f.status}`);
        await onFile(`${rel}${name}`, Buffer.from(await f.arrayBuffer()));
      }
    }
  }
  await walk(`${new URL(base).origin}${hostPath}`, "");
}

/** Export into a tar.gz that is byte-for-byte the same on every run. */
export async function exportRevision(root, path, rev, out, { prefix, ...opts } = {}) {
  const tmp = mkdtempSync(join(tmpdir(), "svn-export-"));
  const top = prefix ?? `${path.split("/").filter(Boolean).pop()}-r${rev}`;
  try {
    let n = 0;
    await listAndFetch(root, path, rev, {
      ...opts,
      onFile: (rel, bytes) => {
        const target = join(tmp, top, rel);
        mkdirSync(dirname(target), { recursive: true });
        writeFileSync(target, bytes);
        n++;
      },
    });
    if (!n) throw new Error("the revision has no files");
    // GNU tar (bundled with Git for Windows): sorted, no owner, no times; gzip without a name or time.
    execFileSync("tar", ["--sort=name", "--mtime=@0", "--owner=0", "--group=0", "--numeric-owner", "-cf", "export.tar", top], { cwd: tmp });
    execFileSync("gzip", ["-9n", "-f", "export.tar"], { cwd: tmp });
    copyFileSync(join(tmp, "export.tar.gz"), out);
    return n;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

if (process.argv[1]?.endsWith("svn-export.mjs")) {
  const [root, path, rev, out, ...rest] = process.argv.slice(2);
  const prefix = rest.includes("--prefix") ? rest[rest.indexOf("--prefix") + 1] : undefined;
  const n = await exportRevision(root, path, rev, out, { anonymous: rest.includes("--anonymous"), prefix });
  console.log(`${n} files -> ${out}`);
}
