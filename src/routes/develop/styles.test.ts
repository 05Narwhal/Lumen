// Develop's look (Round 4J Task 2). Vitest runs with `css: false`, so jsdom computes no
// style from a stylesheet; these read the SCSS itself, which is where the rules live.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const src = join(__dirname, "..", "..");
const read = (p: string) => readFileSync(join(src, p), "utf8").replace(/\r\n/g, "\n");

/** The declarations of the first rule whose selector line is exactly `selector {`. */
function rule(scss: string, selector: string): string {
  const at = scss.indexOf(`${selector} {`);
  if (at < 0) throw new Error(`no rule ${selector}`);
  let depth = 0;
  for (let i = scss.indexOf("{", at); i < scss.length; i++) {
    if (scss[i] === "{") depth++;
    else if (scss[i] === "}" && --depth === 0) return scss.slice(at, i + 1);
  }
  throw new Error(`unclosed rule ${selector}`);
}

const scrolls = (css: string) => /overflow(-y)?:\s*(auto|scroll)/.test(css);

describe("Develop's scrolling", () => {
  it("scrolls only in its columns, never inside a panel or the history list", () => {
    expect(scrolls(read("components/develop-sidebar/styles.scss")), "develop-sidebar").toBe(false);
    expect(scrolls(read("components/develop-panels/styles.scss")), "develop-panels").toBe(false);
    expect(read("components/develop-sidebar/styles.scss")).not.toMatch(/max-height/);
    const develop = read("routes/develop/styles.scss");
    expect(scrolls(rule(develop, "  &__left,\n  &__right"))).toBe(true);
  });

  it("does not sit in the route container's scroller", () => {
    const globals = read("styles/globals.scss");
    const fixed = rule(globals, ".route-transition--fixed");
    expect(fixed).toMatch(/overflow:\s*hidden/);
    expect(fixed).toMatch(/scrollbar-gutter:\s*auto/);
  });
});

describe("Develop's surfaces", () => {
  const develop = read("routes/develop/styles.scss");

  it("lets the page's backdrop through, like every other screen", () => {
    expect(rule(develop, ".develop")).toMatch(/\n  background: transparent;/);
  });

  it("dresses its columns and filmstrip like the shell's sidebar", () => {
    const cols = rule(develop, "  &__left,\n  &__right");
    expect(cols).toMatch(/background: var\(--bg-sidebar\)/);
    expect(cols).toMatch(/backdrop-filter: blur\(var\(--blur\)\)/);
    expect(rule(develop, "  &__strip")).toMatch(/background: var\(--bg-sidebar\)/);
  });

  it("puts the photo on the theme's stage colour", () => {
    expect(rule(develop, "  &__stage")).toMatch(/background: var\(--develop-stage-bg\)/);
  });

  it("titles its sections the way the sidebar does", () => {
    for (const p of ["components/develop-sidebar/styles.scss", "components/develop-panels/styles.scss"]) {
      const title = rule(read(p), "  &__title");
      expect(title, p).toMatch(/text-transform: uppercase/);
      expect(title, p).toMatch(/letter-spacing: 0\.08em/);
    }
  });
});
