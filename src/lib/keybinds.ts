import { isMac } from "@/components/window-controls";
import { appCommands } from "@/lib/commands";

/** The person's own keys by action id; an empty list is an unbound action (Round 19 §3). */
export type KeyOverrides = Record<string, string[]>;

export interface Keybind {
  /** Stable id: a palette command's id, else `scope.action`. Handlers call `matches(e, id)`. */
  id: string;
  /** Default keys as written here: `Mod` is Cmd on macOS and Ctrl elsewhere; `+` joins a chord. */
  keys: string[];
  action: string;
  /** A range or a held key (the rating digits, `Alt` to erase): shown, not rebindable. */
  fixed?: boolean;
}

export interface KeybindGroup {
  id: string;
  title: string;
  /** Where the keys work. */
  where: string;
  binds: Keybind[];
}

/** A registry entry: a bind with the scope (its group's id) it is listed under. */
export interface KeyAction extends Keybind {
  scope: string;
}

/**
 * Keys for commands that had none (Round 19 §3). Chords only, with Ctrl/Cmd: a bare letter in
 * `everywhere` would also fire inside Develop and the editors. Rare or risky commands (reload,
 * restart, eject, the off-switches, settings pages) stay without a key; the palette has them.
 */
export const ADDED_DEFAULTS: Record<string, string> = {
  "go-trash": "Mod+Shift+T",
  "go-hidden": "Mod+Shift+H",
  "go-duplicates": "Mod+Shift+M",
  "go-search": "Mod+Shift+F",
  "toggle-scheme": "Mod+Shift+L",
  "new-collection": "Mod+Shift+N",
  "hide-selection": "Mod+Alt+H",
  "copy-text": "Mod+Alt+C",
  "filter-has-text": "Mod+Alt+T",
  "duplicate-album": "Mod+Alt+D",
  "album-cover-shuffle": "Mod+Alt+S",
  "export-album": "Mod+Alt+E",
  "back-up-now": "Mod+Alt+B",
};

/** The default key a command has: its own `shortcut`, else one added here. */
const shortcutOf = (c: { id: string; shortcut?: string }) => c.shortcut ?? ADDED_DEFAULTS[c.id];

/** Extra default keys beside a command's `shortcut`. */
const ALSO_BY_DEFAULT: Record<string, string[]> = { "tiles-larger": ["Mod+="] };

/**
 * Every keyboard shortcut in the app, for Settings → Keyboard and for `matches`. Commands come
 * from `appCommands` (the palette's list, which the hotkeys also route through), so those
 * cannot drift; the rest are the keys each screen handles itself, listed here beside the
 * handler they document.
 */
export function keybindGroups(): KeybindGroup[] {
  const command = (id: string, action?: string): Keybind => {
    const c = appCommands.find((x) => x.id === id);
    if (!c || !shortcutOf(c)) throw new Error(`command ${id} has no shortcut`);
    return { id, keys: [shortcutOf(c), ...(ALSO_BY_DEFAULT[id] ?? [])], action: action ?? c.label.replace(/^Go to /, "") };
  };
  const fromCommands = (group: string): Keybind[] =>
    appCommands.filter((c) => c.group === group && shortcutOf(c)).map((c) => command(c.id));
  const k = (id: string, keys: string[], action: string, fixed = false): Keybind => ({ id, keys, action, ...(fixed ? { fixed } : {}) });

  const groups: KeybindGroup[] = [
    {
      id: "everywhere",
      title: "Everywhere",
      where: "Anywhere in the app, except while typing",
      binds: [
        k("command-palette", ["Mod+K"], "Command palette"),
        k("search", ["/"], "Search"),
        command("undo-last", "Undo the last action"),
        ...fromCommands("Navigate"),
        ...fromCommands("View"),
      ],
    },
    {
      id: "grid",
      title: "Photo grid",
      where: "Library, albums, search results and other grids",
      binds: [
        k("grid.move", ["←", "→", "↑", "↓"], "Move between photos", true),
        k("grid.extend", ["Shift+←", "Shift+→"], "Extend the selection", true),
        k("grid.open", ["Enter"], "Open the photo"),
        k("grid.quick-look", ["Space"], "Quick look"),
        command("select-all", "Select every photo here"),
        k("grid.clear", ["Escape"], "Clear the selection"),
        command("select-none", "Select none"),
        command("export-selection", "Export the selection"),
        command("develop-selection", "Open the selection in Develop"),
        command("edit-in-default", "Edit in the default app"),
        k("grid.favorite", ["F"], "Add the selection to Favorites"),
        command("rotate-left", "Rotate the selection left"),
        command("rotate-right", "Rotate the selection right"),
        k("grid.trash", ["Delete"], "Move the selection to Trash"),
      ],
    },
    {
      id: "viewer",
      title: "Viewer",
      where: "A photo or video open full size",
      binds: [
        k("viewer.previous", ["←"], "Previous photo"),
        k("viewer.next", ["→"], "Next photo"),
        k("viewer.close", ["Escape"], "Close (leaves full screen first)"),
        k("viewer.rate", ["1", "2", "3", "4", "5"], "Rate", true),
        k("viewer.clear-rating", ["0"], "Clear the rating"),
        k("viewer.favorite", ["F"], "Favorite"),
        k("viewer.info", ["I"], "Info"),
        command("edit-photo", "Edit a photo, trim a video"),
        k("viewer.play-pause", ["Space"], "Play or pause (video, Live Photo, slideshow)"),
        k("viewer.hide-controls", ["Tab"], "Hide the controls"),
        k("viewer.fit", ["Mod+0"], "Fit to window"),
        k("viewer.zoom-1-1", ["Mod+1"], "Actual size"),
        k("viewer.fullscreen", ["F11"], "Full screen"),
        k("viewer.trash", ["Delete"], "Move to Trash"),
      ],
    },
    {
      id: "editor",
      title: "Quick editor",
      where: "Editing a photo",
      binds: [
        k("editor.crop", ["C"], "Crop / done cropping"),
        k("editor.done-crop", ["Enter"], "Done cropping"),
        k("editor.rotate-left", ["["], "Rotate left"),
        k("editor.rotate-right", ["]"], "Rotate right"),
        k("editor.flip-h", ["H"], "Flip horizontal"),
        k("editor.flip-v", ["V"], "Flip vertical"),
        k("editor.grid", ["G"], "Change the grid"),
        k("editor.undo", ["Mod+Z"], "Undo"),
        k("editor.redo", ["Mod+Shift+Z", "Ctrl+Y"], "Redo"),
        k("editor.cancel", ["Escape"], "Cancel (leaves crop first)"),
      ],
    },
    {
      id: "film",
      title: "Memory film",
      where: "Playing a memory",
      binds: [
        k("film.play-pause", ["Space"], "Play or pause"),
        k("film.skip", ["←", "→"], "Skip five seconds", true),
        k("film.mute", ["M"], "Mute"),
        k("film.fullscreen", ["F"], "Full screen"),
        k("film.close", ["Escape"], "Close (leaves full screen first)"),
      ],
    },
    {
      id: "trim",
      title: "Video trimmer",
      where: "Trimming a video",
      binds: [
        k("trim.play-pause", ["Space"], "Play or pause"),
        k("trim.mark-in", ["I"], "Start the selection at the playhead"),
        k("trim.mark-out", ["O"], "End the selection at the playhead"),
        k("trim.cut", ["Delete"], "Cut the selection"),
        k("trim.seek", ["←", "→"], "Seek a second (timeline focused)", true),
        k("trim.seek-5", ["Shift+←", "Shift+→"], "Seek five seconds", true),
        k("trim.ends", ["Home", "End"], "Go to the start / end", true),
        k("trim.undo", ["Mod+Z"], "Undo"),
        k("trim.redo", ["Mod+Shift+Z", "Ctrl+Y"], "Redo"),
        k("trim.cancel", ["Escape"], "Clear the selection, then cancel"),
      ],
    },
    {
      id: "develop",
      title: "Develop",
      where: "In Develop",
      binds: [
        k("develop.previous", ["←"], "Previous photo in the filmstrip"),
        k("develop.next", ["→"], "Next photo in the filmstrip"),
        k("develop.pick", ["P"], "Pick"),
        k("develop.reject", ["X"], "Reject"),
        k("develop.unflag", ["U"], "Unflag"),
        k("develop.labels", ["Alt+1", "Alt+5"], "Colour label, red to purple", true),
        k("develop.label-none", ["Alt+0"], "No colour label"),
        k("develop.before-after", ["\\"], "Before / after (the whole photo)"),
        k("develop.compare-mode", ["Y"], "Before and after: split, side by side, off"),
        k("develop.clipping", ["J"], "Show clipping"),
        k("develop.zoom-toggle", ["Z"], "Fit / the last zoom level"),
        k("develop.zoom-in", ["Mod+=", "Mod++"], "Zoom in"),
        k("develop.zoom-out", ["Mod+-"], "Zoom out"),
        k("develop.fit", ["Mod+0"], "Fit"),
        k("develop.wheel", ["Mod+Wheel"], "Zoom around the pointer", true),
        k("develop.pan", ["Space"], "Hold and drag to move around", true),
        k("develop.select-all", ["Mod+A"], "Select all in the filmstrip"),
        k("develop.select-none", ["Mod+D"], "Select none"),
        k("develop.copy-settings", ["Mod+Shift+C"], "Copy settings…"),
        k("develop.paste-settings", ["Mod+Shift+V"], "Paste settings"),
        command("develop-auto", "Auto tone"),
        command("develop-retouch", "Retouch (when switched on in Settings → Experimental)"),
        k("develop.crop", ["R"], "Crop and straighten / done"),
        k("develop.done-crop", ["Enter"], "Done cropping"),
        k("develop.cancel-crop", ["Escape"], "Cancel the crop's changes since the last step"),
        k("develop.mask-overlay", ["O"], "Show the mask overlay"),
        k("develop.mask-overlay-colour", ["Shift+O"], "Change the mask overlay's colour"),
        k("develop.mask-forward", ["Alt+↑"], "Move the focused mask up the list (applied later)"),
        k("develop.mask-backward", ["Alt+↓"], "Move the focused mask down the list (applied earlier)"),
        k("develop.brush-smaller", ["["], "Brush smaller"),
        k("develop.brush-larger", ["]"], "Brush larger"),
        k("develop.feather-less", ["Shift+["], "Brush feather less"),
        k("develop.feather-more", ["Shift+]"], "Brush feather more"),
        k("develop.brush-drag", ["Alt+RightDrag"], "Drag sideways for brush size, up and down for feather (Ctrl+Option on a Mac)", true),
        k("develop.erase", ["Alt"], "Hold while painting to erase", true),
        k("develop.straight", ["Shift"], "Hold while painting for a straight stroke", true),
        k("develop.undo", ["Mod+Z"], "Undo"),
        k("develop.redo", ["Mod+Shift+Z", "Ctrl+Y"], "Redo"),
      ],
    },
    {
      id: "actions",
      title: "Other actions",
      where: "Anywhere the action applies, except while typing",
      binds: appCommands.filter((c) => c.group !== "Navigate" && c.group !== "View" && !c.shortcut && ADDED_DEFAULTS[c.id]).map((c) => command(c.id)),
    },
    {
      id: "slideshow",
      title: "Slideshow editor",
      where: "Editing a slideshow",
      binds: [
        k("slideshow.play-pause", ["Space"], "Play or pause the preview"),
        k("slideshow.remove-clip", ["Delete"], "Remove the selected clip"),
      ],
    },
  ];

  // The viewer also answers the library's other keys (`P` tag people, `T` text, …), those a
  // group above has not claimed already.
  const claimed = new Set(groups.flatMap((g) => g.binds.map((b) => b.id)));
  const viewer = groups.find((g) => g.id === "viewer")!;
  for (const c of appCommands) {
    if (c.group === "Library" && c.shortcut && !claimed.has(c.id)) viewer.binds.push(command(c.id, c.label.replace(/selection/, "photo")));
  }
  return groups;
}

let registry: KeyAction[] | null = null;

/** Every action that has a key, in the order Settings lists them. Built once. */
export function keyRegistry(): KeyAction[] {
  registry ??= keybindGroups().flatMap((g) => g.binds.map((b) => ({ ...b, scope: g.id })));
  return registry;
}

let overridesNow: KeyOverrides = {};

/** The person's keys, kept where non-React handlers can reach them; the root hook feeds it. */
export function setKeyOverrides(o: KeyOverrides | null | undefined) {
  overridesNow = o ?? {};
}

export const keyOverrides = () => overridesNow;

/** The keys an action has by default. */
export function defaultKeys(id: string): string[] {
  return keyRegistry().find((a) => a.id === id)?.keys ?? [];
}

/** The person's keys for `id`, else its defaults; an unbound action returns `[]`. */
export function keysFor(id: string, overrides: KeyOverrides = overridesNow): string[] {
  return overrides[id] ?? defaultKeys(id);
}

// ---- Chords -------------------------------------------------------------------------------

interface Chord {
  mod: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

const NAMED: Record<string, string> = { "←": "arrowleft", "→": "arrowright", "↑": "arrowup", "↓": "arrowdown", Space: " " };
/** The physical key of each punctuation key, for layouts where Shift changes what it types. */
const PUNCT_CODES: Record<string, string> = {
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "`": "Backquote",
};
const CODE_PUNCT = Object.fromEntries(Object.entries(PUNCT_CODES).map(([c, code]) => [code, c]));
const ARROW_NAMES: Record<string, string> = { ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓" };

const isPunct = (key: string) => key.length === 1 && !/[a-z0-9]/i.test(key);

function parse(spec: string, mac = isMac()): Chord {
  const parts = spec.split(/\+(?!$)/);
  const last = parts.pop() ?? "";
  const chord: Chord = { mod: false, ctrl: false, alt: false, shift: false, key: NAMED[last] ?? last.toLowerCase() };
  for (const p of parts) {
    if (p === "Mod" || (p === "Ctrl" && !mac)) chord.mod = true;
    else if (p === "Ctrl") chord.ctrl = true;
    else if (p === "Alt") chord.alt = true;
    else if (p === "Shift") chord.shift = true;
  }
  return chord;
}

/** One chord as comparable text, so `Ctrl+Y` and `Mod+Y` agree where they mean the same. */
const canonical = (spec: string) => {
  const c = parse(spec);
  return `${c.mod ? "m" : ""}${c.ctrl ? "c" : ""}${c.alt ? "a" : ""}${c.shift ? "s" : ""}|${c.key}`;
};

/** Whether two written keys are the same chord (`Ctrl+Y` and `Mod+Y` are, off macOS). */
export const sameKey = (a: string, b: string) => canonical(a) === canonical(b);

function chordMatches(e: KeyboardEvent, c: Chord, mac = isMac()): boolean {
  if (c.mod !== (mac ? e.metaKey : e.ctrlKey)) return false;
  if (mac ? c.ctrl !== e.ctrlKey : e.metaKey) return false;
  if (c.alt !== e.altKey) return false;
  // A punctuation key is typed with or without Shift depending on the layout (`+`, `?`):
  // Shift only counts for it when the chord names it.
  if ((!isPunct(c.key) || c.shift) && c.shift !== e.shiftKey) return false;
  const typed = ARROW_NAMES[e.key] ? NAMED[ARROW_NAMES[e.key]] : e.key.toLowerCase();
  if (typed === c.key) return true;
  if (c.key === "delete" && typed === "backspace") return true;
  if (c.shift && isPunct(c.key) && e.code === PUNCT_CODES[c.key]) return true;
  // A letter or digit on a layout that types something else (Cyrillic, or Option on a Mac).
  if (/^[a-z0-9]$/.test(c.key) && !/^[a-z0-9]$/i.test(e.key)) return e.code === (/\d/.test(c.key) ? `Digit${c.key}` : `Key${c.key.toUpperCase()}`);
  return false;
}

/** Whether `e` is one of the keys now set for action `id`. This is what handlers call. */
export function matches(e: KeyboardEvent, id: string, overrides: KeyOverrides = overridesNow): boolean {
  return keysFor(id, overrides).some((spec) => chordMatches(e, parse(spec)));
}

/** A pressed key written as the registry writes it, or null for a modifier on its own. */
export function eventToKey(e: KeyboardEvent, mac = isMac()): string | null {
  if (["Shift", "Control", "Alt", "Meta", "AltGraph", "Dead", "Unidentified", "CapsLock"].includes(e.key)) return null;
  if (!mac && e.metaKey) return null;
  let key: string;
  let shift = e.shiftKey;
  if (ARROW_NAMES[e.key]) key = ARROW_NAMES[e.key];
  else if (e.key === " ") key = "Space";
  else if (e.key.length > 1) key = e.key; // Escape, Enter, Delete, F11, Home…
  else if (/^[a-z0-9]$/i.test(e.key)) key = e.key.toUpperCase();
  else if (/^Key[A-Z]$/.test(e.code) || /^Digit\d$/.test(e.code)) key = e.code.slice(-1); // Option or a non-Latin layout
  else if (e.key === "+") {
    key = "+";
    shift = false;
  } else if (PUNCT_CODES[e.key]) {
    key = e.key;
    shift = false;
  } else if (CODE_PUNCT[e.code]) key = CODE_PUNCT[e.code]; // `{` is Shift+`[`
  else key = e.key;
  const mods = [mac ? (e.metaKey ? "Mod" : "") : e.ctrlKey ? "Mod" : "", mac && e.ctrlKey ? "Ctrl" : "", e.altKey ? "Alt" : "", shift ? "Shift" : ""].filter(Boolean);
  return [...mods, key].join("+");
}

/**
 * Another action that already has `key` in the same scope or everywhere (Round 19 §3), judged
 * with the person's own keys; null when the key is free or only `id` itself uses it.
 */
export function findConflict(id: string, key: string, overrides: KeyOverrides = overridesNow): KeyAction | null {
  const target = keyRegistry().find((a) => a.id === id);
  if (!target) return null;
  const want = canonical(key);
  return (
    keyRegistry().find(
      (a) =>
        a.id !== id &&
        (a.scope === target.scope || a.scope === "everywhere" || target.scope === "everywhere") &&
        keysFor(a.id, overrides).some((k) => canonical(k) === want),
    ) ?? null
  );
}

/** One key as shown: `Mod` becomes ⌘ or Ctrl, and chords keep their `+`. */
export function formatKey(key: string, mac = isMac()): string[] {
  return key.split(/\+(?!$)/).map((k) => (k === "Mod" ? (mac ? "⌘" : "Ctrl") : k === "Shift" && mac ? "⇧" : k.length === 1 ? k.toUpperCase() : k));
}
