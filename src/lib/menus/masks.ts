import { aiLabel } from "@/lib/ai-masks";
import type { AiMaskInfo } from "@/types/generated/AiMaskInfo";
import type { AiMaskKind } from "@/types/generated/AiMaskKind";
import type { AiMaskList } from "@/types/generated/AiMaskList";
import { IconArrowBarToDown, IconArrowBarToUp, IconArrowDown, IconArrowUp, IconCopy, IconEye, IconEyeOff, IconFlipVertical, IconPencil, IconRestore, IconStack2, IconTrash } from "@tabler/icons-react";
import { adjustChanged } from "@/lib/develop-masks";
import type { Mask } from "@/types/generated/Mask";
import type { MenuItems } from "@/components/context-menu";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export interface MaskActions {
  rename: () => void;
  duplicate: () => void;
  invert: () => void;
  toggleVisible: () => void;
  /** Move to the array index (later = applied later = top of the list). */
  moveTo: (index: number) => void;
  resetAdjust: () => void;
  remove: () => void;
}

/**
 * One mask's menu (Round 21 §3), for its row's right-click and the bar's ⋯. The list reads like
 * layers, so *front* is the end of the array: `index` is the mask's place in it, `count` its length.
 */
export function maskMenu(mask: Mask, index: number, count: number, a: MaskActions): MenuItems {
  const last = count - 1;
  const items: MenuItems = [
    { label: "Rename", icon: icon(IconPencil), onSelect: a.rename },
    { label: "Duplicate", icon: icon(IconCopy), onSelect: a.duplicate },
    { label: mask.invert ? "Invert ✓" : "Invert", icon: icon(IconFlipVertical), onSelect: a.invert },
    mask.visible ? { label: "Hide", icon: icon(IconEyeOff), onSelect: a.toggleVisible } : { label: "Show", icon: icon(IconEye), onSelect: a.toggleVisible },
    DIVIDER,
    {
      label: "Arrange",
      icon: icon(IconStack2),
      children: [
        { label: "Bring to front", icon: icon(IconArrowBarToUp), disabled: index >= last, onSelect: () => a.moveTo(last) },
        { label: "Bring forward", icon: icon(IconArrowUp), disabled: index >= last, onSelect: () => a.moveTo(index + 1) },
        { label: "Send backward", icon: icon(IconArrowDown), disabled: index <= 0, onSelect: () => a.moveTo(index - 1) },
        { label: "Send to back", icon: icon(IconArrowBarToDown), disabled: index <= 0, onSelect: () => a.moveTo(0) },
      ],
    },
    DIVIDER,
    { label: "Reset adjustments", icon: icon(IconRestore), disabled: !adjustChanged(mask.adjust), onSelect: a.resetAdjust },
    { label: "Delete", icon: icon(IconTrash), danger: true, onSelect: a.remove },
  ];
  items.dense = true;
  return items;
}

export interface AiMaskActions {
  /** Use a mask that was found. */
  pick: (m: AiMaskInfo) => void;
  /** Not scanned yet: find the masks, then use the first of `kind`. */
  find: (kind: AiMaskKind) => void;
  /** Look at the photo again, even though Facet already has. */
  again: () => void;
}

const SINGLE: { kind: Exclude<AiMaskKind, "person">; label: string; none: string }[] = [
  { kind: "sky", label: "Sky", none: "No sky found" },
  { kind: "subject", label: "Subject", none: "No subject found" },
  { kind: "background", label: "Background", none: "No background found" },
];

const nothing = () => {};

/**
 * The AI part of *New mask ▸*, *Add ▸* and *Subtract ▸* (Phase 8f): Sky, Subject,
 * Background and People ▸ Person 1…n. Before Facet has looked, each item finds first; after,
 * what wasn't found is disabled and says so, and *Detect again* looks once more.
 */
export function aiMaskMenu(list: AiMaskList, a: AiMaskActions): MenuItem[] {
  if (!list.scanned) {
    return [...SINGLE.map((s) => ({ label: s.label, onSelect: () => a.find(s.kind) })), { label: "People", onSelect: () => a.find("person") }];
  }
  const people = list.masks.filter((m) => m.kind === "person").sort((x, y) => x.instance - y.instance);
  return tidy([
    ...SINGLE.map((s): MenuItem => {
      const m = list.masks.find((x) => x.kind === s.kind);
      return m ? { label: s.label, onSelect: () => a.pick(m) } : { label: s.none, disabled: true, onSelect: nothing };
    }),
    people.length > 0
      ? { label: "People", children: people.map((p) => ({ label: aiLabel(p), onSelect: () => a.pick(p) })) }
      : { label: "No people found", disabled: true, onSelect: nothing },
    DIVIDER,
    { label: "Detect again", onSelect: a.again },
  ]);
}
