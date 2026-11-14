// Filled buttons (the photo picker's "Add 3" / "Develop 3", every primary action) must be
// readable in every theme and colour scheme: WCAG AA, 4.5:1, for the text on the accent
// and on its hover state (Round 4A). The ratios are computed from the theme partials
// themselves. (The picker's unreadable "Develop 3" was a stylesheet greying the label,
// pinned in photo-picker's test; the tokens passed. This keeps them passing.)
//
// The pairs and thresholds live in `contrast-golden.json`, written by the photos-plugins theme
// tests (Rust is the reference and also checks plugin themes against the same list). This file
// reads them, computes the ratios from the partials, and checks Rust's sample ratios for every
// colour form a plugin theme may use against its own maths.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import golden from "./contrast-golden.json";

const THEMES = join(__dirname, "themes");

/** The custom properties of one rule block, by name. */
function block(scss: string, selector: string): Record<string, string> {
  const at = scss.indexOf(`${selector} {`);
  if (at < 0) return {};
  const body = scss.slice(at + selector.length + 2, scss.indexOf("}", at));
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/(--[\w-]+):\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

/** `oklch(L% C H [/ a])` → linear sRGB, clamped to the gamut. */
function oklchToLinear(v: string): [number, number, number] {
  const m = /oklch\(\s*([\d.]+)%\s+([\d.]+)\s+([\d.]+)/.exec(v);
  if (!m) throw new Error(`not an oklch colour: ${v}`);
  const L = Number(m[1]) / 100;
  const C = Number(m[2]);
  const h = (Number(m[3]) * Math.PI) / 180;
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);
  const l_ = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m_ = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s_ = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const rgb: [number, number, number] = [
    4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
    -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
    -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
  ];
  return rgb.map((c) => Math.min(1, Math.max(0, c))) as [number, number, number];
}

const decodeSrgb = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** Any colour form a plugin theme may use (`#hex`, `rgb()`, `hsl()`, `oklch()`) → linear sRGB. */
function toLinear(v: string): [number, number, number] {
  const s = v.trim();
  if (s.startsWith("oklch(")) return oklchToLinear(s);
  if (s.startsWith("#")) {
    const h = s.slice(1).length <= 4 ? [...s.slice(1)].map((c) => c + c).join("") : s.slice(1);
    return [0, 1, 2].map((i) => decodeSrgb(parseInt(h.slice(i * 2, i * 2 + 2), 16) / 255)) as [number, number, number];
  }
  const m = /^(rgba?|hsla?)\(([^)]*)\)$/.exec(s);
  if (!m) throw new Error(`not a colour: ${v}`);
  const parts = m[2].split("/")[0].split(/[\s,]+/).filter(Boolean);
  if (m[1].startsWith("rgb")) {
    return parts.slice(0, 3).map((p) => decodeSrgb(Math.round(Number(p.replace("%", "")) * (p.endsWith("%") ? 2.55 : 1)) / 255)) as [number, number, number];
  }
  const h = Number(parts[0].replace("deg", ""));
  const sat = Number(parts[1].replace("%", "")) / 100;
  const lig = Number(parts[2].replace("%", "")) / 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = sat * Math.min(lig, 1 - lig);
  const f = (n: number) => lig - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0), f(8), f(4)].map(decodeSrgb) as [number, number, number];
}

const luminance = (v: string) => {
  const [r, g, b] = toLinear(v);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const themes = readdirSync(THEMES)
  .filter((f) => f.endsWith(".scss"))
  .map((f) => ({ name: f.replace(/^_|\.scss$/g, ""), scss: readFileSync(join(THEMES, f), "utf8") }));

describe("the formula", () => {
  it("knows it: black on white is 21:1", () => {
    expect(contrast("oklch(0% 0 0)", "oklch(100% 0 0)")).toBeCloseTo(21, 1);
  });

  // Rust turns every colour form a plugin theme may use (hex, rgb, hsl, oklch) into linear sRGB;
  // its ratios against white must be the ones this file's maths gives.
  for (const s of golden.samples) {
    it(`agrees with Rust on ${s.fg}`, () => {
      expect(contrast(s.fg, s.bg)).toBeCloseTo(s.ratio, 2);
    });
  }
});

/** `oklch(... / a)` → its alpha, 1 when opaque. */
const alpha = (v: string) => Number(/\/\s*([\d.]+)\s*\)/.exec(v)?.[1] ?? 1);

/** Linear sRGB → encoded sRGB, the space alpha compositing happens in. */
const encode = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
const decode = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** The luminance of `top` painted over the opaque `under`, as the browser composites it. */
function luminanceOver(top: string, under: string): number {
  const a = alpha(top);
  const t = toLinear(top).map(encode);
  const u = toLinear(under).map(encode);
  const [r, g, b] = t.map((c, i) => decode(c * a + u[i] * (1 - a)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Contrast of text `fg` on `surface`, both laid over the page's opaque `base`. */
function textContrast(fg: string, surface: string, base: string): number {
  const s = luminanceOver(surface, base);
  // Text is opaque ink in every theme; a translucent one would composite over the surface.
  const f = alpha(fg) === 1 ? luminance(fg) : s;
  const [hi, lo] = [f, s].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// Every pair in the golden, in every theme and scheme: the label on the accent and its hover, a
// disabled button, Develop's stage, every ink that sets words on every surface (HIG Accessibility
// › Vision), and the accent as text. `--fg-faint` is exempt: it draws hairlines, tracks and
// unavailable controls, never words.
describe("contrast pairs", () => {
  it("has pairs", () => {
    expect(golden.pairs.length).toBeGreaterThan(10);
  });
  for (const { name, scss } of themes) {
    const light = block(scss, `[data-theme="${name}"]`);
    const dark = { ...light, ...block(scss, `[data-theme="${name}"][data-mantine-color-scheme="dark"]`) };
    for (const [scheme, t] of [["light", light], ["dark", dark]] as const) {
      it(`${name} ${scheme}: --develop-stage-bg and --fg-faint are defined`, () => {
        expect(t["--develop-stage-bg"], "--develop-stage-bg is defined").toBeDefined();
        expect(t["--fg-faint"], "--fg-faint is defined").toBeDefined();
      });
      for (const p of golden.pairs) {
        it(`${name} ${scheme}: ${p.fg} on ${p.bg} reads at ${p.min}:1`, () => {
          const got = p.overBase
            ? textContrast(t[p.fg], t[p.bg], t["--bg-base"])
            : contrast(t[p.fg], t[p.bg]);
          expect(got).toBeGreaterThanOrEqual(p.min);
        });
      }
      it(`${name} ${scheme}: --fg-faint is fainter than --fg-subtle`, () => {
        const base = t["--bg-base"];
        expect(textContrast(t["--fg-faint"], base, base)).toBeLessThan(textContrast(t["--fg-subtle"], base, base));
      });
    }
  }
});

// Mantine draws a `light` or `subtle` button with no colour from the primary palette's own
// variables (`--mantine-color-blue-light-color`, …), not the `--mantine-primary-color-*` ones,
// so "Review" on Home was Mantine's stock blue in every theme. One accent means one thing.
describe("Mantine's primary palette", () => {
  it("is the theme's accent for light, subtle and outline buttons", async () => {
    const { cssVariablesResolver } = await import("./mantine-theme");
    const v = cssVariablesResolver({} as never);
    for (const scheme of [v.light, v.dark]) {
      expect(scheme["--mantine-color-blue-light"]).toBe("var(--accent-soft)");
      expect(scheme["--mantine-color-blue-light-color"]).toBe("var(--accent)");
      expect(scheme["--mantine-color-blue-outline"]).toBe("var(--accent)");
      expect(scheme["--mantine-color-blue-filled"]).toBe("var(--accent)");
    }
  });
});

describe("Mantine's disabled colours", () => {
  it("come from the theme, not Mantine's grey palette", async () => {
    const { cssVariablesResolver } = await import("./mantine-theme");
    const v = cssVariablesResolver({} as never);
    for (const scheme of [v.light, v.dark]) {
      expect(scheme["--mantine-color-disabled"]).toBe("var(--bg-sunken)");
      expect(scheme["--mantine-color-disabled-color"]).toBe("var(--fg-muted)");
    }
  });
});
