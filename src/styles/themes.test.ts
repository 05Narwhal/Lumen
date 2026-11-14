import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { THEMES } from "@/types/theme";
import { contrastRatio, oklchToRgb, parseOklch } from "@/utils/color";

function blocks(id: string) {
  const src = readFileSync(join(process.cwd(), "src/styles/themes", `_${id}.scss`), "utf8");
  const out: Record<string, Record<string, string>> = {};
  for (const m of src.matchAll(
    /\[data-theme="(\w+)"\](\[data-mantine-color-scheme="(\w+)"\])?\s*\{([^}]*)\}/g,
  )) {
    const scheme = m[3] ?? "light";
    const vars: Record<string, string> = {};
    for (const v of m[4].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) vars[v[1]] = v[2].trim();
    out[scheme] = vars;
  }
  return out;
}

const rgb = (css: string) => {
  const p = parseOklch(css);
  if (!p) throw new Error(`not oklch: ${css}`);
  return oklchToRgb(p.l, p.c, p.h);
};

const COLOUR_TOKENS = [
  "--bg-base", "--bg-raised", "--bg-sunken", "--bg-sidebar", "--bg-overlay",
  "--fg-primary", "--fg-muted", "--fg-subtle", "--fg-faint",
  "--accent", "--accent-hover", "--accent-fg", "--accent-soft",
  "--border", "--scrollbar-thumb", "--shadow-sm", "--shadow-md", "--shadow-lg",
];
const SHAPE_TOKENS = [
  "--border-width", "--radius-sm", "--radius-md", "--radius-lg", "--radius-pill", "--tile-gap",
  "--blur", "--font-heading", "--heading-weight", "--heading-style", "--wash",
];

describe.each(THEMES.map((t) => t.id))("theme %s", (id) => {
  const b = blocks(id);

  it.each(["light", "dark"])("%s scheme meets AA for primary and muted text", (scheme) => {
    const v = b[scheme];
    expect(v, `missing ${scheme} block`).toBeDefined();
    const bg = rgb(v["--bg-base"]);
    expect(contrastRatio(rgb(v["--fg-primary"]), bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(rgb(v["--fg-muted"]), bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(rgb(v["--accent-fg"]), rgb(v["--accent"]))).toBeGreaterThanOrEqual(3);
  });

  it("defines every themeable token in both schemes", () => {
    for (const scheme of ["light", "dark"]) {
      for (const k of COLOUR_TOKENS) expect(b[scheme][k], `${scheme} ${k}`).toBeDefined();
    }
    for (const k of SHAPE_TOKENS) expect(b.light[k], k).toBeDefined();
  });
});
