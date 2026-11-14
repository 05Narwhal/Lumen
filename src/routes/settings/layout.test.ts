import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fixedPage } from "@/routes/root-layout";

// Round 18 B1: the sidebar and the page scroll on their own, under a header that stays put,
// and the sections fill the width in columns. jsdom has no layout, so these guard the CSS and
// the one line that stops the whole route from scrolling as one.
describe("settings layout", () => {
  const scss = readFileSync(join(process.cwd(), "src/routes/settings/styles.scss"), "utf8");
  const rule = (selector: string) => scss.match(new RegExp(`${selector.replace(/[&_.-]/g, "\\$&")}\\s*\\{([^}]*)\\}`))?.[1] ?? "";

  it("makes Settings a fixed page, so only its own panes scroll", () => {
    expect(fixedPage("/settings")).toBe(true);
    expect(fixedPage("/settings?section=music")).toBe(true);
    expect(fixedPage("/albums")).toBe(false);
  });

  it("fills the window and clips, so the header and the title bar never scroll away", () => {
    const root = scss.match(/^\.settings\s*\{([^}]*)\}/m)?.[1] ?? "";
    expect(root).toMatch(/height:\s*100%/);
    expect(root).toMatch(/overflow:\s*hidden/);
  });

  it("scrolls the sidebar and the page body each on their own", () => {
    expect(rule("&__nav")).toMatch(/overflow-y:\s*auto/);
    expect(rule("&__body")).toMatch(/overflow-y:\s*auto/);
  });

  it("lets a sidebar entry's tag give way before its name does", () => {
    const nav = readFileSync(join(process.cwd(), "src/components/settings-nav/styles.scss"), "utf8");
    const navRule = (selector: string) => nav.match(new RegExp(`${selector.replace(/[&_.-]/g, "\\$&")}\\s*\\{([^}]*)\\}`))?.[1] ?? "";
    expect(navRule("&__label")).toMatch(/flex:\s*0 0 auto/);
    expect(navRule("&__status")).toMatch(/flex:\s*0 1 auto/);
    expect(navRule("&__status")).toMatch(/min-width:\s*0/);
    expect(navRule("&__status")).toMatch(/margin-left:\s*auto/);
  });

  it("puts one column of rows beside a sticky inspector (sample B)", () => {
    expect(rule("&__split")).toMatch(/grid-template-columns:\s*minmax\(0, 1fr\) minmax\(260px/);
    expect(scss).toMatch(/\.settings-inspector\s*\{\s*position:\s*sticky/);
  });
});
