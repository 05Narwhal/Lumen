/**
 * The tone curve's shape, for drawing it: a monotone cubic (Fritsch–Carlson) through the
 * points, (0, 0) and (1, 1) implied when absent, continuing along its end tangents. Rust
 * renders with the same maths (`photos-media::develop::curve`), and `curve-golden.json`,
 * which a Rust test writes, holds this to it. Never used on pixels.
 */
export type CurvePoint = [number, number];

export function monotone(points: CurvePoint[]): (x: number) => number {
  const p = [...points].sort((a, b) => a[0] - b[0]);
  if (p.length === 0 || p[0][0] > 1e-4) p.unshift([0, 0]);
  if (p[p.length - 1][0] < 1 - 1e-4) p.push([1, 1]);
  const xs = p.map((q) => q[0]);
  const ys = p.map((q) => q[1]);
  const n = xs.length;
  const d = xs.slice(0, -1).map((x, k) => (ys[k + 1] - ys[k]) / Math.max(xs[k + 1] - x, 1e-6));
  const ms = new Array<number>(n).fill(0);
  ms[0] = d[0];
  ms[n - 1] = d[n - 2];
  for (let k = 1; k < n - 1; k++) ms[k] = d[k - 1] * d[k] <= 0 ? 0 : (d[k - 1] + d[k]) / 2;
  for (let k = 0; k < n - 1; k++) {
    if (d[k] === 0) {
      ms[k] = 0;
      ms[k + 1] = 0;
      continue;
    }
    const a = ms[k] / d[k];
    const b = ms[k + 1] / d[k];
    const h = a * a + b * b;
    if (h > 9) {
      const t = 3 / Math.sqrt(h);
      ms[k] = t * a * d[k];
      ms[k + 1] = t * b * d[k];
    }
  }
  return (x: number) => {
    if (x <= xs[0]) return ys[0] + (x - xs[0]) * ms[0];
    if (x >= xs[n - 1]) return ys[n - 1] + (x - xs[n - 1]) * ms[n - 1];
    let k = 0;
    while (k < n - 2 && xs[k + 1] <= x) k++;
    const h = xs[k + 1] - xs[k];
    const t = (x - xs[k]) / h;
    const t2 = t * t;
    const t3 = t2 * t;
    return (
      (2 * t3 - 3 * t2 + 1) * ys[k] + (t3 - 2 * t2 + t) * h * ms[k] + (-2 * t3 + 3 * t2) * ys[k + 1] + (t3 - t2) * h * ms[k + 1]
    );
  };
}
