import { IconClipboardCopy, IconClipboardText, IconColumns2, IconDownload, IconFlag, IconRestore, IconTag, IconTriangleInverted, IconZoomIn } from "@tabler/icons-react";
import type { CompareMode } from "@/hooks/use-develop-session";
import { DIVIDER, icon, openWithItems, tidy, type MenuItem, type OpenWithContext } from "./parts";

export type DevelopZoom = "fit" | "fill" | 1 | 2;

export interface DevelopCanvasContext {
  compare: CompareMode;
  clipping: boolean;
  /** Colour labels in Lightroom's order: `value` 1 red … 5 purple. */
  labels: readonly { value: number; name: string }[];
  /** The installed apps that fit the open photo, and the default (Round 22 §9). */
  openWith?: OpenWithContext;
}

/** The canvas's own controls and the workspace's actions on the open photo. */
export interface DevelopCanvasActions {
  setCompare: (mode: CompareMode) => void;
  setClipping: (on: boolean) => void;
  zoom: (to: DevelopZoom) => void;
  copySettings: () => void;
  pasteSettings: () => void;
  reset: () => void;
  /** −1 reject, 0 none, 1 pick. */
  flag: (value: -1 | 0 | 1) => void;
  label: (value: number) => void;
  export: () => void;
  /** *Edit in* and *Open with ▸* for the open photo. */
  openWith?: { editIn: (appId: string) => void; chooseApp: () => void };
}

/**
 * Right-click on Develop's canvas (Round 4G): Before / after ▸ · Clipping · Zoom ▸ · — · Copy
 * settings · Paste settings · Reset develop · — · Flag ▸ · Label ▸ · Export…. Crop and masks
 * join when Phase 8e has built them.
 */
export function developCanvasMenu(ctx: DevelopCanvasContext, a: DevelopCanvasActions): MenuItem[] {
  const mode = (m: CompareMode, label: string, shortcut?: string): MenuItem => ({
    label: ctx.compare === m ? `${label} ✓` : label,
    shortcut,
    onSelect: () => a.setCompare(ctx.compare === m ? "off" : m),
  });
  return tidy([
    {
      label: "Before / after",
      icon: icon(IconColumns2),
      children: [mode("toggle", "Toggle", "\\"), mode("split", "Split", "Y"), mode("side", "Side by side")],
    },
    { label: ctx.clipping ? "Hide clipping" : "Show clipping", icon: icon(IconTriangleInverted), shortcut: "J", onSelect: () => a.setClipping(!ctx.clipping) },
    {
      label: "Zoom",
      icon: icon(IconZoomIn),
      children: [
        { label: "Fit", shortcut: "Z", onSelect: () => a.zoom("fit") },
        { label: "Fill", onSelect: () => a.zoom("fill") },
        { label: "100 %", onSelect: () => a.zoom(1) },
        { label: "200 %", onSelect: () => a.zoom(2) },
      ],
    },
    DIVIDER,
    { label: "Copy settings…", icon: icon(IconClipboardCopy), shortcut: "Mod+Shift+C", onSelect: a.copySettings },
    { label: "Paste settings", icon: icon(IconClipboardText), shortcut: "Mod+Shift+V", onSelect: a.pasteSettings },
    { label: "Reset develop", icon: icon(IconRestore), onSelect: a.reset },
    DIVIDER,
    {
      label: "Flag",
      icon: icon(IconFlag),
      children: [
        { label: "Pick", shortcut: "P", onSelect: () => a.flag(1) },
        { label: "Reject", shortcut: "X", onSelect: () => a.flag(-1) },
        { label: "Unflag", shortcut: "U", onSelect: () => a.flag(0) },
      ],
    },
    {
      label: "Label",
      icon: icon(IconTag),
      children: [
        ...ctx.labels.map((l) => ({ label: l.name, shortcut: `Alt+${l.value}`, onSelect: () => a.label(l.value) })),
        { label: "None", shortcut: "Alt+0", onSelect: () => a.label(0) },
      ],
    },
    { label: "Export…", icon: icon(IconDownload), onSelect: a.export },
    ...(ctx.openWith && a.openWith ? [DIVIDER, ...openWithItems(ctx.openWith, a.openWith)] : []),
  ]);
}
