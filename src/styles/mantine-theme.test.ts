import { describe, expect, it } from "vitest";
import { FLOATING_MIDDLEWARES, mantineTheme } from "./mantine-theme";

/**
 * Round 5: dropdowns ran off the window. Every floating surface takes the same middlewares
 * from the theme, so no single dropdown has to remember them.
 */
describe("floating surfaces", () => {
  const props = (name: string) => (mantineTheme.components?.[name]?.defaultProps ?? {}) as Record<string, unknown>;

  it("flip to the side with room, even when neither side fits whole", () => {
    expect(FLOATING_MIDDLEWARES.flip).toMatchObject({ fallbackStrategy: "bestFit" });
  });

  it("shift back inside the window on both axes and cap their size to the room left", () => {
    expect(FLOATING_MIDDLEWARES.shift).toMatchObject({ crossAxis: true });
    expect(FLOATING_MIDDLEWARES.size).toBeTruthy();
  });

  it.each(["Popover", "Menu", "HoverCard"])("%s uses them", (name) => {
    expect(props(name).middlewares).toBe(FLOATING_MIDDLEWARES);
  });

  it.each(["Select", "MultiSelect", "Autocomplete", "TagsInput", "Combobox"])("%s uses them", (name) => {
    const p = props(name);
    const combobox = (p.comboboxProps ?? p) as Record<string, unknown>;
    expect(combobox.middlewares).toBe(FLOATING_MIDDLEWARES);
  });
});
