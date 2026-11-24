#!/usr/bin/env node
// Finds personal data in the files that would be published. See the open-source plan, Task 7.
//   node scripts/oss/scrub.mjs [--words pro-cloud] [--no-local] [--no-exclude] [--stdin] <paths…>
// Prints `path:line rule` for each finding and exits 1 when there is any.
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const IMAGE = /\.(png|jpe?g|gif|webp|heic|heif|avif|tiff?|bmp|ico|icns)$/i;
const EXIF_IMAGE = /\.(jpe?g|png|heic|heif|tiff?)$/i;

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function globToRegExp(glob) {
  const g = glob.replace(/\\/g, "/");
  const anywhere = !g.includes("/");
  let out = "";
  for (let i = 0; i < g.length; i++) {
    const c = g[i];
    if (c === "*" && g[i + 1] === "*") {
      if (g[i + 2] === "/") {
        out += "(?:.*/)?";
        i += 2;
      } else {
        out += ".*";
        i += 1;
      }
    } else if (c === "*") out += "[^/]*";
    else if (c === "?") out += "[^/]";
    else out += escapeRe(c);
  }
  return new RegExp(`^${anywhere ? "(?:.*/)?" : ""}${out}$`);
}

function matchesAny(path, globs) {
  const p = path.replace(/\\/g, "/");
  return globs.some((g) => globToRegExp(g).test(p));
}

export function selectFiles(files, excludes) {
  return files.filter((f) => !matchesAny(f, excludes));
}

export function loadRules(publicPath, localPath) {
  const pub = JSON.parse(readFileSync(publicPath, "utf8"));
  const rules = {
    deny: pub.deny ?? [],
    allow: pub.allow ?? [],
    words: pub.words ?? {},
    mediaAllow: [...(pub.mediaAllow ?? [])],
    skipText: pub.skipText ?? [],
    exifAllow: pub.exifAllow ?? [],
    localDeny: [],
    activeWords: [],
  };
  if (localPath && existsSync(localPath)) {
    const loc = JSON.parse(readFileSync(localPath, "utf8"));
    rules.localDeny = (loc.deny ?? []).map(String);
    rules.mediaAllow.push(...(loc.mediaAllow ?? []));
  }
  return rules;
}

function lineRules(rules) {
  const list = [];
  for (const src of rules.deny) list.push({ rule: `deny:${src}`, re: new RegExp(src, "gi"), allowable: true });
  rules.localDeny.forEach((s, i) =>
    list.push({ rule: `deny:local-${i + 1}`, re: new RegExp(escapeRe(s), "gi") }),
  );
  for (const name of rules.activeWords ?? []) {
    for (const w of rules.words[name] ?? []) list.push({ rule: `words:${name}`, re: new RegExp(escapeRe(w), "gi") });
  }
  const allow = (rules.allow ?? []).map((a) => new RegExp(a, "i"));
  return { list, allow };
}

export function scanText(path, text, rules) {
  const { list, allow } = lineRules(rules);
  const findings = [];
  text.split(/\r?\n/).forEach((line, i) => {
    for (const r of list) {
      r.re.lastIndex = 0;
      let hit = false;
      let m;
      while ((m = r.re.exec(line))) {
        if (m[0] === "") {
          r.re.lastIndex++;
          continue;
        }
        if (r.allowable && allow.some((a) => a.test(m[0]))) continue;
        hit = true;
        break;
      }
      if (hit) findings.push({ path, line: i + 1, rule: r.rule });
    }
  });
  return findings;
}

// ---- EXIF: reads only the TIFF block ----

function tiffTags(buf, base) {
  const found = new Set();
  if (base + 8 > buf.length) return found;
  const order = buf.toString("latin1", base, base + 2);
  const le = order === "II";
  if (!le && order !== "MM") return found;
  const u16 = (o) => (o + 2 <= buf.length ? (le ? buf.readUInt16LE(o) : buf.readUInt16BE(o)) : 0);
  const u32 = (o) => (o + 4 <= buf.length ? (le ? buf.readUInt32LE(o) : buf.readUInt32BE(o)) : 0);
  const seen = new Set();
  const ifd = (off, level) => {
    if (seen.has(off) || base + off + 2 > buf.length) return;
    seen.add(off);
    const n = Math.min(u16(base + off), 512);
    for (let i = 0; i < n; i++) {
      const e = base + off + 2 + i * 12;
      if (e + 12 > buf.length) break;
      const tag = u16(e);
      const type = u16(e + 2);
      const count = u32(e + 4);
      if (tag === 0x8825 && level === "ifd0") found.add("GPS");
      if (tag === 0x8769 && level === "ifd0") ifd(u32(e + 8), "exif");
      if (level === "ifd0" && tag === 0x013b && type === 2 && count > 1) found.add("Artist");
      if (level === "ifd0" && tag === 0x8298 && type === 2 && count > 1) found.add("Copyright");
      if (level === "exif" && tag === 0xa431 && type === 2 && count > 1) found.add("BodySerialNumber");
    }
  };
  ifd(u32(base + 4), "ifd0");
  return found;
}

function exifBlocks(bytes) {
  const tiffs = [];
  if (bytes[0] === 0xff && bytes[1] === 0xd8) {
    let o = 2;
    while (o + 4 <= bytes.length && bytes[o] === 0xff) {
      const marker = bytes[o + 1];
      if (marker === 0xd9 || marker === 0xda) break;
      const len = bytes.readUInt16BE(o + 2);
      if (marker === 0xe1 && bytes.toString("latin1", o + 4, o + 10) === "Exif\0\0") tiffs.push(o + 10);
      o += 2 + len;
    }
  } else if (bytes.toString("latin1", 1, 4) === "PNG") {
    let o = 8;
    while (o + 12 <= bytes.length) {
      const len = bytes.readUInt32BE(o);
      if (bytes.toString("latin1", o + 4, o + 8) === "eXIf") tiffs.push(o + 8);
      o += 12 + len;
    }
  } else {
    const head = bytes.toString("latin1");
    for (const sig of ["II*\0", "MM\0*"]) {
      const i = head.indexOf(sig);
      if (i >= 0) tiffs.push(i);
    }
  }
  return tiffs;
}

export function scanFile(path, bytes, rules) {
  const p = path.replace(/\\/g, "/");
  if (/\.pyc$/i.test(p) || p.split("/").includes("__pycache__")) return [{ path, rule: "pycache" }];
  const findings = scanText(path, p, rules).map(({ rule }) => ({ path, rule }));
  if (IMAGE.test(p)) {
    if (!matchesAny(p, rules.mediaAllow)) return [...findings, { path, rule: "media-not-allowed" }];
    if (EXIF_IMAGE.test(p) && !matchesAny(p, rules.exifAllow ?? [])) {
      const tags = new Set();
      for (const b of exifBlocks(bytes)) for (const t of tiffTags(bytes, b)) tags.add(t);
      for (const t of ["GPS", "Artist", "Copyright", "BodySerialNumber"]) {
        if (tags.has(t)) findings.push({ path, rule: `exif:${t}` });
      }
    }
    return findings;
  }
  if (matchesAny(p, rules.skipText ?? [])) return findings; // the rules file names the patterns
  if (bytes.subarray(0, 8192).includes(0)) return findings; // binary: the path only
  return [...findings, ...scanText(path, bytes.toString("utf8"), rules)];
}

// ---- CLI ----

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    if (name === ".git" || name === "node_modules") continue;
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) walk(full, out);
    else if (st.isFile()) out.push(relative(process.cwd(), full).split(sep).join("/"));
  }
}

function main(argv) {
  const opts = { words: [], stdin: false, local: true, exclude: [join(here, "public-exclude.txt")], paths: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--words") opts.words.push(argv[++i]);
    else if (a === "--stdin") opts.stdin = true;
    else if (a === "--no-local") opts.local = false;
    else if (a === "--no-exclude") opts.exclude = [];
    else if (a === "--exclude") opts.exclude.push(argv[++i]);
    else opts.paths.push(a);
  }
  const rules = loadRules(
    join(here, "scrub-rules.json"),
    opts.local ? join(here, "scrub-rules.local.json") : undefined,
  );
  rules.activeWords = opts.words;
  const globs = opts.exclude.flatMap((f) =>
    readFileSync(f, "utf8")
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#")),
  );
  let files = [];
  if (opts.stdin) files.push(...readFileSync(0, "utf8").split(/\r?\n/).filter(Boolean));
  for (const p of opts.paths) {
    if (statSync(p).isDirectory()) walk(p, files);
    else files.push(p.replace(/\\/g, "/"));
  }
  files = selectFiles(files, globs);
  const counts = new Map();
  for (const f of files) {
    if (!existsSync(f)) continue;
    for (const x of scanFile(f, readFileSync(f), rules)) {
      console.log(`${x.path}${x.line ? ":" + x.line : ""} ${x.rule}`);
      counts.set(x.path, (counts.get(x.path) ?? 0) + 1);
    }
  }
  const total = [...counts.values()].reduce((a, b) => a + b, 0);
  console.error(`${files.length} files scanned, ${total} findings in ${counts.size} files`);
  return total ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main(process.argv.slice(2)));
}
