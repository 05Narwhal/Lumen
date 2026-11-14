import { describe, expect, it } from "vitest";
import { appCommands } from "@/lib/commands";
import { ADDED_DEFAULTS, eventToKey, findConflict, formatKey, keybindGroups, keyRegistry, keysFor, matches } from "./keybinds";

describe("keybindGroups", () => {
  it("lists every command that has a shortcut", () => {
    // Letter keys are case-insensitive: the viewer's `e` is listed as E.
    const listed = new Set(keybindGroups().flatMap((g) => g.binds.flatMap((b) => b.keys.map((k) => k.toLowerCase()))));
    for (const c of appCommands.filter((c) => c.shortcut)) expect(listed, c.id).toContain(c.shortcut!.toLowerCase());
  });

  // Round 4J: Develop's zoom and before-and-after keys are listed in Settings → Keyboard.
  it("lists Develop's zoom and compare keys", () => {
    const develop = keybindGroups().find((g) => g.id === "develop")!;
    const keys = develop.binds.flatMap((b) => b.keys);
    for (const k of ["Y", "Z", "Mod+=", "Mod+-", "Mod+0", "Space"]) expect(keys, k).toContain(k);
  });

  // Round 8B: Escape leaves a video's or a film's full screen before it closes anything.
  it("says Escape leaves full screen first, in the viewer and in a memory's film", () => {
    const groups = keybindGroups();
    const esc = (id: string) => groups.find((g) => g.id === id)?.binds.find((b) => b.keys.includes("Escape"))?.action;
    expect(esc("viewer")).toMatch(/full screen first/);
    expect(esc("film")).toMatch(/full screen first/);
  });

  it("names each group once", () => {
    const ids = keybindGroups().map((g) => g.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("formatKey", () => {
  it("writes Mod for the platform and splits chords, keeping a literal +", () => {
    expect(formatKey("Mod+Shift+Z", false)).toEqual(["Ctrl", "Shift", "Z"]);
    expect(formatKey("Mod+Shift+Z", true)).toEqual(["⌘", "⇧", "Z"]);
    expect(formatKey("Mod++", false)).toEqual(["Ctrl", "+"]);
    expect(formatKey("[", false)).toEqual(["["]);
  });
});

const ev = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

describe("keyRegistry", () => {
  it("gives every action one stable id", () => {
    const ids = keyRegistry().map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ["go-library", "grid.open", "viewer.zoom-1-1", "develop.crop"]) expect(ids, id).toContain(id);
  });

  it("scopes each action to the group that lists it", () => {
    const by = (id: string) => keyRegistry().find((a) => a.id === id)!;
    expect(by("go-library").scope).toBe("everywhere");
    expect(by("grid.open").scope).toBe("grid");
    expect(by("develop.crop").scope).toBe("develop");
  });
});

describe("keysFor", () => {
  it("returns the default, the person's override, or nothing when unbound", () => {
    expect(keysFor("go-library", {})).toEqual(["2"]);
    expect(keysFor("go-library", { "go-library": ["Mod+L"] })).toEqual(["Mod+L"]);
    expect(keysFor("go-library", { "go-library": [] })).toEqual([]);
    expect(keysFor("no-such-id", {})).toEqual([]);
  });
});

describe("matches", () => {
  it("reads Mod as Ctrl here and needs it to be held", () => {
    expect(matches(ev({ key: "k", ctrlKey: true }), "command-palette", {})).toBe(true);
    expect(matches(ev({ key: "k" }), "command-palette", {})).toBe(false);
    expect(matches(ev({ key: "k", ctrlKey: true, shiftKey: true }), "command-palette", {})).toBe(false);
  });

  it("is case-insensitive for letters but exact about Shift", () => {
    expect(matches(ev({ key: "f" }), "grid.favorite", {})).toBe(true);
    expect(matches(ev({ key: "F", shiftKey: true }), "grid.favorite", {})).toBe(false);
  });

  it("takes shifted punctuation by its key or its physical key", () => {
    expect(matches(ev({ key: "+", ctrlKey: true, shiftKey: true }), "tiles-larger", {})).toBe(true);
    expect(matches(ev({ key: "=", ctrlKey: true }), "tiles-larger", {})).toBe(true);
    expect(matches(ev({ key: "[", ctrlKey: true }), "rotate-left", {})).toBe(true);
    const o = { "develop.brush-softer": ["Shift+["] };
    expect(matches(ev({ key: "{", code: "BracketLeft", shiftKey: true }), "develop.brush-softer", o)).toBe(true);
    expect(matches(ev({ key: "[", code: "BracketLeft" }), "develop.brush-softer", o)).toBe(false);
  });

  it("follows the physical key on a layout whose letters are not Latin", () => {
    expect(matches(ev({ key: "а", code: "KeyF" }), "grid.favorite", {})).toBe(true);
  });

  it("names arrows, Space and Delete the way the registry does", () => {
    expect(matches(ev({ key: "ArrowLeft", altKey: true }), "go-back", {})).toBe(true);
    expect(matches(ev({ key: "ArrowLeft" }), "go-back", {})).toBe(false);
    expect(matches(ev({ key: " " }), "grid.quick-look", {})).toBe(true);
    expect(matches(ev({ key: "Delete" }), "grid.trash", {})).toBe(true);
    expect(matches(ev({ key: "Backspace" }), "grid.trash", {})).toBe(true);
  });

  it("uses the override instead of the default, and never matches an unbound action", () => {
    const o = { "go-library": ["Mod+L"], "go-map": [] };
    expect(matches(ev({ key: "2" }), "go-library", o)).toBe(false);
    expect(matches(ev({ key: "l", ctrlKey: true }), "go-library", o)).toBe(true);
    expect(matches(ev({ key: "4" }), "go-map", o)).toBe(false);
  });
});

describe("eventToKey", () => {
  it("writes a pressed chord the way the registry does", () => {
    expect(eventToKey(ev({ key: "l", ctrlKey: true }))).toBe("Mod+L");
    expect(eventToKey(ev({ key: "ArrowLeft", shiftKey: true }))).toBe("Shift+←");
    expect(eventToKey(ev({ key: "{", code: "BracketLeft", shiftKey: true }))).toBe("Shift+[");
    expect(eventToKey(ev({ key: " " }))).toBe("Space");
    expect(eventToKey(ev({ key: "Shift", shiftKey: true }))).toBeNull();
  });
});

describe("findConflict", () => {
  it("finds another action on the same key in the same scope or everywhere", () => {
    expect(findConflict("viewer.info", "F", {})?.id).toBe("viewer.favorite");
    expect(findConflict("grid.open", "Mod+A", {})?.id).toBe("select-all");
    expect(findConflict("grid.open", "Enter", {})).toBeNull(); // its own key is no clash
    // An action everywhere clashes with a screen's; two different screens never do.
    expect(findConflict("go-library", "F", {})?.id).toBe("grid.favorite");
    expect(findConflict("grid.favorite", "I", {})).toBeNull();
  });

  it("sees the person's own keys, and not a key that was freed", () => {
    expect(findConflict("grid.open", "Mod+L", { "go-library": ["Mod+L"] })?.id).toBe("go-library");
    expect(findConflict("viewer.info", "F", { "viewer.favorite": [] })).toBeNull();
  });
});

describe("added defaults", () => {
  it("binds every added key once, to a real command, with no clash", () => {
    const reg = keyRegistry();
    for (const [id, key] of Object.entries(ADDED_DEFAULTS)) {
      expect(appCommands.some((c) => c.id === id), id).toBe(true);
      expect(reg.filter((a) => a.id === id)).toHaveLength(1);
      expect(findConflict(id, key), `${id} ${key}`).toBeNull();
    }
  });
});
