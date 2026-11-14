import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The licences screen reads `src/assets/licenses/notices.json`, which
 * `scripts/licenses/generate.py` writes before every release build. These tests fail when it
 * has drifted from the dependencies, so a new crate or package cannot ship uncredited.
 */
const root = join(__dirname, "../..");
const file = join(root, "src/assets/licenses/notices.json");

interface Item {
  name: string;
  version?: string;
  license: string;
  texts: string[];
  note?: string;
}
interface Notices {
  texts: Record<string, string>;
  groups: { id: string; title: string; items: Item[] }[];
}

const notices = (): Notices => JSON.parse(readFileSync(file, "utf8"));
const names = (n: Notices, group: string) => new Set(n.groups.find((g) => g.id === group)?.items.map((i) => i.name) ?? []);

/** Dependency names from a `[dependencies]`-style table of a Cargo.toml. */
function cargoDeps(path: string, tables: string[]): string[] {
  const out: string[] = [];
  let inTable = false;
  for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
    const header = line.match(/^\s*\[([^\]]+)\]/);
    if (header) {
      inTable = tables.includes(header[1].trim());
      continue;
    }
    const dep = inTable && line.match(/^\s*([A-Za-z0-9_-]+)\s*=/);
    // Path dependencies are Lumen's own crates.
    if (dep && !/path\s*=/.test(line)) out.push(dep[1]);
  }
  return out;
}

describe("third-party notices", () => {
  it("exists and every entry's texts are present", () => {
    expect(existsSync(file)).toBe(true);
    const n = notices();
    for (const g of n.groups) for (const i of g.items) for (const t of i.texts) expect(n.texts[t], `${g.id}/${i.name} → ${t}`).toBeTruthy();
  });

  it("lists Lumen's own licence first", () => {
    const first = notices().groups[0].items[0];
    expect(first.name).toBe("Lumen");
    expect(first.license).toBe("GPL-3.0-or-later");
  });

  it("lists every production npm dependency", () => {
    const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    const npm = names(notices(), "npm");
    for (const d of Object.keys(pkg.dependencies)) expect(npm.has(d), d).toBe(true);
  });

  it("lists every crate Lumen depends on directly", () => {
    const rust = names(notices(), "rust");
    const tauri = join(root, "src-tauri");
    const tomls = [join(tauri, "Cargo.toml"), ...readdirSync(join(tauri, "crates")).map((c) => join(tauri, "crates", c, "Cargo.toml"))];
    const deps = new Set(tomls.flatMap((t) => cargoDeps(t, ["dependencies", "workspace.dependencies", "build-dependencies"])));
    for (const d of deps) expect(rust.has(d), d).toBe(true);
  });

  it("carries what the licences require to be shown", () => {
    const n = notices();
    const rust = n.groups.find((g) => g.id === "rust")!.items;
    const rawler = rust.find((i) => i.name === "rawler")!;
    expect(rawler.license).toBe("LGPL-2.1");
    expect(rawler.texts.length).toBeGreaterThan(0);
    for (const [group, name] of [
      ["programs", "FFmpeg"],
      ["programs", "ExifTool"],
      ["data", "OpenStreetMap data"],
      ["data", "GeoNames"],
      ["notices", "Independent JPEG Group"],
      ["models", "clip-vit-b32-fp16@1"],
      ["npm", "@fontsource-variable/inter"],
    ])
      expect(names(n, group).has(name), `${group}/${name}`).toBe(true);
  });

  // The source is public, so there is no written offer; the Licences page links to the repository.
  it("carries no written offer of source", () => {
    const items = notices().groups.flatMap((g) => g.items);
    expect(items.find((i) => i.name === "Source code offer")).toBeUndefined();
    expect(JSON.stringify(notices())).not.toContain("lumen-releases");
  });
});
