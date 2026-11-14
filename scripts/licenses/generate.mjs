// Writes src/assets/licenses/notices.json: every third-party work Lumen ships or shows, with
// its licence texts, for Settings → Licences. Run by `yarn licenses` and before every release
// build (`yarn build`), so it never drifts; src/lib/licenses.test.ts checks it.
//
// Sources: the Rust crates of the shipped graph on Windows, macOS and Linux (normal and build
// dependencies of the workspace, never dev-only ones), the production npm packages, the ML
// models in photos-ml's manifest, and scripts/licenses/extra.json for everything else.
import { execFileSync, execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gplCompatible } from "./compat.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const here = join(root, "scripts/licenses");
const out = join(root, "src/assets/licenses/notices.json");
const TARGETS = ["x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu"];
const LICENSE_FILE = /^(licen[cs]e|copying|copyright|notice|unlicense)([-._].*)?$/i;

/** Licence texts, deduplicated by content: id → text. */
const texts = {};
function addText(text) {
  const t = text.replace(/\r\n/g, "\n").trim();
  if (!t) return null;
  const id = createHash("sha256").update(t).digest("hex").slice(0, 12);
  texts[id] = t;
  return id;
}
/** A standard SPDX text from texts/, under its own id. */
function spdx(id) {
  const p = join(here, "texts", `${id}.txt`);
  if (!existsSync(p)) return null;
  texts[id] = readFileSync(p, "utf8").replace(/\r\n/g, "\n").trim();
  return id;
}
/** The licence files at the top of a package folder. */
function filesIn(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => addText(readFileSync(join(dir, f), "utf8")))
    .filter(Boolean);
}
/** When a package ships no file: the standard texts its SPDX expression names, where known. */
function fallback(license) {
  const ids = String(license ?? "")
    .split(/\s+(?:OR|AND|WITH)\s+|\/|[()]/)
    .map((s) => s.trim())
    .filter(Boolean);
  return [...new Set(ids.map((id) => spdx(id === "LGPL-2.1" ? "LGPL-2.1-only" : id)).filter(Boolean))];
}

// Rust: the union of the shipped graphs.
const crates = new Map();
for (const target of TARGETS) {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", target, "--manifest-path", join(root, "src-tauri/Cargo.toml")], {
      maxBuffer: 1 << 28,
      encoding: "utf8",
    }),
  );
  const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const workspace = new Set(meta.workspace_members);
  const seen = new Set();
  const stack = [...workspace];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id).deps) if (d.dep_kinds.some((k) => k.kind === null || k.kind === "build")) stack.push(d.pkg);
  }
  for (const id of seen) {
    if (workspace.has(id)) continue;
    const p = pkgs.get(id);
    const key = `${p.name}@${p.version}`;
    if (crates.has(key)) continue;
    const dir = dirname(p.manifest_path);
    const own = filesIn(dir);
    crates.set(key, {
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? "See licence file" : "Unknown"),
      url: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
      texts: own.length ? own : fallback(p.license),
    });
  }
}

// npm: production packages, with the licence files in node_modules.
// One fixed command string: `yarn` is a .cmd shim on Windows, which only a shell can start.
const listed = execSync("yarn licenses list --prod --json --no-progress", { cwd: root, encoding: "utf8", maxBuffer: 1 << 26, stdio: ["ignore", "pipe", "ignore"] });
const table = listed
  .split(/\r?\n/)
  .filter(Boolean)
  .map((l) => {
    try {
      return JSON.parse(l);
    } catch {
      return null;
    }
  })
  .find((o) => o?.type === "table");
const npm = table.data.body.map(([name, version, license, url]) => {
  const own = filesIn(join(root, "node_modules", name));
  return { name, version, license, url: url && url !== "Unknown" ? url : `https://www.npmjs.com/package/${name}`, texts: own.length ? own : fallback(license) };
});

// Models: the manifest says each one's licence; THIRD-PARTY-MODELS.md says the rest.
const manifest = JSON.parse(readFileSync(join(root, "src-tauri/crates/photos-models/models.json"), "utf8"));
const models = manifest.models.map((m) => {
  const lic = m.licence ?? "See THIRD-PARTY-MODELS.md";
  const ids = [/\bMIT\b/.test(lic) && spdx("MIT"), /Apache/.test(lic) && spdx("Apache-2.0")].filter(Boolean);
  return { name: m.id, version: m.file, license: lic, texts: ids };
});

const byName = (a, b) => a.name.localeCompare(b.name) || String(a.version).localeCompare(String(b.version));
const extra = JSON.parse(readFileSync(join(here, "extra.json"), "utf8"));
for (const g of extra.groups) for (const i of g.items) i.texts = i.texts.map(spdx).filter(Boolean);
// Lumen's own entry carries this build's version, so it never drifts from package.json.
const lumen = extra.groups.find((g) => g.id === "lumen")?.items.find((i) => i.name === "Lumen");
if (lumen) lumen.version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;

const notices = {
  generated: new Date().toISOString().slice(0, 10),
  texts,
  groups: [
    ...extra.groups.filter((g) => g.id === "lumen"),
    ...extra.groups.filter((g) => g.id === "programs"),
    { id: "models", title: "Machine-learning models", intro: "Models Lumen downloads for its smart features. Training data and sources are listed in THIRD-PARTY-MODELS.md.", items: models },
    ...extra.groups.filter((g) => g.id !== "programs" && g.id !== "lumen"),
    { id: "rust", title: "Rust libraries", intro: "Compiled into Lumen.", items: [...crates.values()].sort(byName) },
    { id: "npm", title: "JavaScript libraries and fonts", intro: "Part of Lumen's interface.", items: npm.sort(byName) },
  ],
};
// The gate: everything compiled into Lumen or shipped in its interface must be usable in a
// GPL-3.0-or-later program. Models are data and the programs in extra.json run separately, so
// neither is judged here.
const refused = [...crates.values(), ...npm].map((i) => ({ i, r: gplCompatible(i.license) })).filter((x) => !x.r.ok);
if (refused.length) {
  console.error(`${refused.length} dependenc${refused.length === 1 ? "y has" : "ies have"} a licence a GPL-3.0-or-later program can't include:`);
  for (const { i, r } of refused) console.error(`  ${i.name}@${i.version}  [${i.license}]  ${r.reason}`);
  process.exit(1);
}
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${JSON.stringify(notices)}\n`);
const count = notices.groups.reduce((n, g) => n + g.items.length, 0);
console.log(`notices.json: ${count} entries, ${Object.keys(texts).length} distinct texts, ${(JSON.stringify(notices).length / 1024).toFixed(0)} KB`);
