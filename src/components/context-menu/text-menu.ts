// Cut, Copy, Paste and Select all in every text field (Round 4G). The webview's own menu is
// off in a release build, so without this a text field had no right-click at all. A field
// inside a card or a row shows this menu, never the object's.

import type { MenuItem } from "./gui";

/** Input types whose text can be selected and edited (a number field has no selection API). */
const TEXT_TYPES = new Set(["text", "search", "email", "url", "tel", "password"]);

export type TextField = HTMLInputElement | HTMLTextAreaElement;
export type Editable = TextField | HTMLElement;

/** The editable element a right-click landed in, if any. */
export function editableAt(target: EventTarget | null): Editable | null {
  if (!(target instanceof Element)) return null;
  const field = target.closest("input, textarea, [contenteditable]");
  if (field instanceof HTMLInputElement) return TEXT_TYPES.has(field.type) ? field : null;
  if (field instanceof HTMLTextAreaElement) return field;
  if (field instanceof HTMLElement && field.isContentEditable) return field;
  return null;
}

const isField = (el: Editable): el is TextField => el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;

/** The clipboard, through the webview's `navigator.clipboard`. */
export interface TextClipboard {
  readText: () => Promise<string>;
  writeText: (text: string) => Promise<void>;
}

/** What is selected in `el` when the menu opens; the field loses focus to the menu after. */
export interface Snapshot {
  start: number;
  end: number;
  selected: string;
}

export function snapshot(el: Editable): Snapshot {
  if (isField(el)) {
    const start = el.selectionStart ?? 0;
    const end = el.selectionEnd ?? start;
    return { start, end, selected: el.value.slice(start, end) };
  }
  return { start: 0, end: 0, selected: window.getSelection()?.toString() ?? "" };
}

/** Replaces `start..end` of a field with `text` the way typing would, so React's `onChange` runs. */
export function insertText(el: TextField, text: string, start: number, end: number) {
  el.focus();
  el.setRangeText(text, start, end, "end");
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Cut · Copy · Paste · Select all (and Undo · Redo above them in a textarea). Items that
 * cannot apply are disabled: Cut and Copy without a selection, Paste with nothing to paste.
 * A read-only field offers only Copy and Select all; a password is never copied out.
 */
export function textMenu(el: Editable, sel: Snapshot, clip: TextClipboard, pasteText: string | null): MenuItem[] {
  const field = isField(el);
  const readOnly = field ? el.readOnly || el.disabled : false;
  const secret = el instanceof HTMLInputElement && el.type === "password";
  const has = sel.selected.length > 0;
  const items: MenuItem[] = [];
  if (el instanceof HTMLTextAreaElement && !readOnly) {
    items.push(
      { label: "Undo", shortcut: "Mod+Z", onSelect: () => (el.focus(), document.execCommand("undo")) },
      { label: "Redo", shortcut: "Mod+Shift+Z", onSelect: () => (el.focus(), document.execCommand("redo")) },
      { divider: true },
    );
  }
  if (!readOnly) {
    items.push({
      label: "Cut",
      shortcut: "Mod+X",
      disabled: !has || secret,
      onSelect: () =>
        void clip.writeText(sel.selected).then(() => {
          if (field) insertText(el, "", sel.start, sel.end);
          else (el.focus(), document.execCommand("delete"));
        }),
    });
  }
  items.push({ label: "Copy", shortcut: "Mod+C", disabled: !has || secret, onSelect: () => void clip.writeText(sel.selected) });
  if (!readOnly) {
    const paste = (text: string) => {
      if (!text) return;
      if (field) insertText(el, text, sel.start, sel.end);
      else (el.focus(), document.execCommand("insertText", false, text));
    };
    items.push({
      label: "Paste",
      shortcut: "Mod+V",
      // Disabled only when the clipboard is known to hold no text; when it could not be read
      // in time, Paste reads it when chosen.
      disabled: pasteText === "",
      onSelect: () => (pasteText ? paste(pasteText) : void clip.readText().then(paste, () => {})),
    });
  }
  items.push({ divider: true });
  items.push({
    label: "Select all",
    shortcut: "Mod+A",
    onSelect: () => {
      el.focus();
      if (field) el.select();
      else window.getSelection()?.selectAllChildren(el);
    },
  });
  return items;
}

/** The webview's clipboard, or one that holds nothing where there is none (tests, old webviews). */
export function systemClipboard(): TextClipboard {
  const c = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
  return {
    readText: () => (c ? c.readText() : Promise.resolve("")),
    writeText: (t) => (c ? c.writeText(t) : Promise.resolve()),
  };
}

/**
 * What can be pasted, without holding the menu up: the text, `""` when the clipboard holds
 * none, or `null` when it could not be read within `ms` (or was refused).
 */
export async function pastable(clip: TextClipboard, ms = 150): Promise<string | null> {
  const timeout = new Promise<null>((r) => setTimeout(() => r(null), ms));
  try {
    return await Promise.race([clip.readText(), timeout]);
  } catch {
    return null;
  }
}
