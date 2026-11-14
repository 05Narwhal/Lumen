export type Rgb = [number, number, number];

/** Parse `oklch(L% C H)` / `oklch(L C H / A)` into components (L in 0–1). */
export function parseOklch(css: string) {
  const m = css
    .trim()
    .match(/^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)(?:\s*\/\s*([\d.]+%?))?\s*\)$/i);
  if (!m) return null;
  const lRaw = parseFloat(m[1]);
  const l = m[2] === "%" ? lRaw / 100 : lRaw;
  const alpha = m[5] === undefined ? 1 : m[5].endsWith("%") ? parseFloat(m[5]) / 100 : parseFloat(m[5]);
  return { l, c: parseFloat(m[3]), h: parseFloat(m[4]), alpha };
}

/** OKLCH → sRGB (0–255, clamped). */
export function oklchToRgb(l: number, c: number, h: number): Rgb {
  const hr = (h * Math.PI) / 180;
  const a = c * Math.cos(hr);
  const b = c * Math.sin(hr);
  const l_ = l + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = l - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = l - 0.0894841775 * a - 1.291485548 * b;
  const L = l_ ** 3;
  const M = m_ ** 3;
  const S = s_ ** 3;
  const lin: Rgb = [
    +4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S,
    -1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S,
    -0.0041960863 * L - 0.7034186147 * M + 1.707614701 * S,
  ];
  return lin.map((v) => {
    const x = Math.min(1, Math.max(0, v));
    const srgb = x <= 0.0031308 ? 12.92 * x : 1.055 * Math.pow(x, 1 / 2.4) - 0.055;
    return Math.round(srgb * 255);
  }) as Rgb;
}

export function relativeLuminance([r, g, b]: Rgb): number {
  const f = (v: number) => {
    const x = v / 255;
    return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** WCAG 2 contrast ratio, 1–21. */
export function contrastRatio(fg: Rgb, bg: Rgb): number {
  const a = relativeLuminance(fg);
  const b = relativeLuminance(bg);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
