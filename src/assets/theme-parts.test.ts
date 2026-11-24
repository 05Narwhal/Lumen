import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import parts from "./theme-parts.json";

/** Every `.ts` / `.tsx` file under `src/`, tests and generated types left out. */
function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (name !== "generated") sources(path, out);
    } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) out.push(path);
  }
  return out;
}

describe("theme parts (THEMES.md)", () => {
  const text = sources(join(__dirname, "..")).map((f) => readFileSync(f, "utf8")).join("\n");
  // `data-part="x"`, `"data-part": "x"` (a Mantine default prop) or `data-part={"x"}`.
  const marked = (part: string) => new RegExp(`data-part["']?\\s*[:=]\\s*\\{?\\s*["']${part}["']`).test(text);

  it("every documented part is hooked in the app", () => {
    expect(parts.parts.filter((p) => !marked(p))).toEqual([]);
  });

  it("every protected part is marked, so no rule can reach inside it", () => {
    expect(parts.protected.filter((p) => !marked(p))).toEqual([]);
  });
});
