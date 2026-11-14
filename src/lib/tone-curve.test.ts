import { describe, expect, it } from "vitest";
import golden from "./curve-golden.json";
import { monotone, type CurvePoint } from "./tone-curve";

/** The curve editor draws what the render does: Rust writes the answers, we check them. */
describe("tone curve against the Rust golden file", () => {
  it("has cases", () => {
    expect(golden.cases.length).toBeGreaterThan(0);
  });

  for (const [i, c] of golden.cases.entries()) {
    it(`case ${i} matches Rust at every sample`, () => {
      const f = monotone(c.points as CurvePoint[]);
      golden.xs.forEach((x, j) => expect(f(x)).toBeCloseTo(c.ys[j], 5));
    });
  }

  it("is the identity with no points", () => {
    const f = monotone([]);
    for (const x of [0, 0.3, 1]) expect(f(x)).toBeCloseTo(x, 6);
  });
});
