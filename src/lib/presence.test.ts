import { describe, expect, it } from "vitest";
import { screenFor } from "./presence";

const idle = { film: false, slideshowPlaying: false };
const SCREENS = ["browsing", "viewing", "developing", "slideshow"];

describe("screenFor", () => {
  it("maps the four screens", () => {
    expect(screenFor({ pathname: "/library", search: "", ...idle })).toBe("browsing");
    expect(screenFor({ pathname: "/library", search: "?asset=9", ...idle })).toBe("viewing");
    expect(screenFor({ pathname: "/develop/4", search: "", ...idle })).toBe("developing");
    expect(screenFor({ pathname: "/slideshows/2", search: "", film: false, slideshowPlaying: true })).toBe("slideshow");
    expect(screenFor({ pathname: "/memories", search: "", film: true, slideshowPlaying: false })).toBe("slideshow");
  });

  it("counts everything else as browsing", () => {
    for (const p of ["/", "/albums", "/map", "/people/3", "/settings", "/search", "/trash"]) {
      expect(screenFor({ pathname: p, search: "", ...idle })).toBe("browsing");
    }
  });

  it("lets no id, name or query reach Discord", () => {
    const inputs: [string, string][] = [
      ["/albums/Secret%20Album", ""],
      ["/people/42", ""],
      ["/library/group/Lisbon%202023", ""],
      ["/search", "?q=mum"],
      ["/develop/1234", ""],
      ["/library", "?asset=99&album=Holiday"],
    ];
    for (const [pathname, search] of inputs) {
      const out = screenFor({ pathname, search, ...idle });
      expect(SCREENS).toContain(out);
      expect(JSON.stringify(out)).not.toMatch(/Secret|42|Lisbon|mum|1234|99|Holiday/);
    }
  });
});
