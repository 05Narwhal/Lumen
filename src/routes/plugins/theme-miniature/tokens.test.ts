import { describe, expect, it } from "vitest";
import type { ThemeSwatch } from "@/types/generated/ThemeSwatch";
import { miniatureVars, scaleLength } from "./tokens";

const swatch: ThemeSwatch = {
  bg: "oklch(98% 0.004 85)",
  surface: "#ffffff",
  sidebar: "rgb(240 238 232)",
  fg: "oklch(20% 0.01 85)",
  fg2: "oklch(45% 0.01 85)",
  accent: "oklch(58% 0.14 40)",
  placeholder: "oklch(88% 0.01 85)",
  radius: "12px",
  gap: "6px",
};

describe("theme miniature tokens", () => {
  it("maps every swatch field to its own custom property", () => {
    const v = miniatureVars(swatch);
    expect(v["--mini-bg"]).toBe(swatch.bg);
    expect(v["--mini-surface"]).toBe(swatch.surface);
    expect(v["--mini-sidebar"]).toBe(swatch.sidebar);
    expect(v["--mini-fg"]).toBe(swatch.fg);
    expect(v["--mini-fg2"]).toBe(swatch.fg2);
    expect(v["--mini-accent"]).toBe(swatch.accent);
    expect(v["--mini-placeholder"]).toBe(swatch.placeholder);
  });

  it("scales px lengths down to fit the small window, with a floor", () => {
    expect(scaleLength("12px")).toBe("5px");
    expect(scaleLength("6px")).toBe("2px");
    expect(scaleLength("0px")).toBe("0px");
    expect(scaleLength("1px", 0.4, 1)).toBe("1px");
  });

  it("keeps the theme's own shape: a square theme stays square, a round one rounds", () => {
    expect(miniatureVars({ ...swatch, radius: "0px" })["--mini-radius"]).toBe("0px");
    expect(miniatureVars({ ...swatch, radius: "24px" })["--mini-radius"]).toBe("10px");
  });

  it("falls back to a safe length when a size isn't plain px, and never lets text through", () => {
    expect(scaleLength("calc(1px + 2px)")).toBe("4px");
    expect(miniatureVars({ ...swatch, gap: "var(--x)" })["--mini-gap"]).toBe("3px");
  });
});
