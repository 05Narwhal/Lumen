import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// Round 9B: the Keyboard box ran far below its last group because the settings row around
// it was a *wrapping* column flexbox, which sized the list's height at one 340 px column.
// Neither a grid (8B) nor columns changed that. jsdom has no layout, so these guard the
// CSS, and `scripts/ui-checks/keyboard-layout.mjs` measures it in a real browser.
describe("keybind list layout", () => {
  const read = (p: string) => readFileSync(join(process.cwd(), p), "utf8");
  const list = read("src/components/keybind-list/styles.scss");
  const settings = read("src/routes/settings/styles.scss");

  it("never wraps a wide settings row", () => {
    const wide = settings.match(/&--wide\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(wide).toMatch(/flex-direction:\s*column/);
    expect(wide).toMatch(/flex-wrap:\s*nowrap/);
  });

  it("packs whole groups in columns", () => {
    const groups = list.match(/&__groups\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(groups).toMatch(/(^|\s)columns:/);
    expect(list).toMatch(/&__group\s*\{[^}]*break-inside:\s*avoid/);
  });
});
