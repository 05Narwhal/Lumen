// Menus for the library's objects other than photos and albums (Round 4G, Task 2).

import {
  IconAperture,
  IconArrowsJoin,
  IconCircleCheck,
  IconClipboard,
  IconCopy,
  IconDownload,
  IconExternalLink,
  IconEye,
  IconEyeOff,
  IconFolderOpen,
  IconMoodEmpty,
  IconPaw,
  IconPawOff,
  IconPencil,
  IconPhotoStar,
  IconMovie,
  IconPlayerPlay,
  IconRefresh,
  IconSlideshow,
  IconTag,
  IconTrash,
  IconUserCircle,
  IconUserOff,
  IconUserPlus,
  IconX,
} from "@tabler/icons-react";
import type { AssetFace } from "@/types/generated/AssetFace";
import type { AssetPet } from "@/types/generated/AssetPet";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import type { PetSummary } from "@/types/generated/PetSummary";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export interface MemoryMenuActions {
  open: () => void;
  /** The memory's film (Phase F's `memory_film`); left out until F has merged. */
  play?: () => void;
  /** Saves the film as a slideshow (Phase F's `save_memory_film`); left out until F has merged. */
  makeSlideshow?: () => void;
  /** Round 6: the film as an MP4, with no slideshow made. */
  exportVideo?: () => void;
  exportPhotos: () => void;
  setHidden: (hidden: boolean) => void;
}

/** A memory card: Play · Open · Make slideshow · Export video… · Export photos… · — · Hide memory (undo). */
export function memoryMenu(memory: MemorySummary, a: MemoryMenuActions): MenuItem[] {
  return tidy([
    a.play && { label: "Play", icon: icon(IconPlayerPlay), onSelect: a.play },
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    a.makeSlideshow && { label: "Make slideshow", icon: icon(IconSlideshow), onSelect: a.makeSlideshow },
    a.exportVideo && { label: "Export video…", icon: icon(IconMovie), onSelect: a.exportVideo },
    { label: "Export photos…", icon: icon(IconDownload), onSelect: a.exportPhotos },
    DIVIDER,
    memory.hidden
      ? { label: "Show memory again", icon: icon(IconEye), onSelect: () => a.setHidden(false) }
      : { label: "Hide memory", icon: icon(IconEyeOff), onSelect: () => a.setHidden(true) },
  ]);
}

export interface PersonMenuActions {
  open: () => void;
  /** Start choosing several people with this one picked. */
  select?: () => void;
  rename: () => void;
  merge?: () => void;
  /** Merging needs someone else to merge with. */
  canMerge?: boolean;
  /** Phase I's groups: a submenu and *Remove from {group}*, built by the page. */
  groups?: MenuItem[];
  notAPerson: () => void;
  remove: () => void;
}

/**
 * A person (or, once Phase C exists, a pet) on the People page: Open · Select · Rename · Merge
 * with… · groups · — · Not a person · Remove from People. Both last two are undone from their
 * toast; *Remove* keeps the faces, *Not a person* rejects them.
 */
export function personMenu(p: PersonSummary, a: PersonMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    a.select && { label: "Select", icon: icon(IconCircleCheck), shortcut: "Ctrl+Click", onSelect: a.select },
    { label: p.name ? "Rename" : "Add a name", icon: icon(IconPencil), onSelect: a.rename },
    a.merge && { label: "Merge with…", icon: icon(IconArrowsJoin), disabled: a.canMerge === false, onSelect: a.merge },
    ...(a.groups ?? []),
    DIVIDER,
    { label: "Not a person", icon: icon(IconMoodEmpty), onSelect: a.notAPerson },
    { label: "Remove from People", icon: icon(IconUserOff), danger: true, onSelect: a.remove },
  ]);
}

export interface FaceMenuActions {
  tag: () => void;
  newPerson: () => void;
  /** *Not {name}*: only for a face given to someone. */
  untag?: () => void;
  /** *Use as main photo of {name}*: only for a face given to someone. */
  useAsMain?: () => void;
  /** *Not a face*: only for a detected face (a drawn one is removed with *Not {name}*). */
  notAFace?: () => void;
}

/** A face box in the viewer (Phase I's *Show faces*): Tag… · New person… · Not {name} · Use as main photo of {name} · Not a face. */
export function faceMenu(face: AssetFace, a: FaceMenuActions): MenuItem[] {
  const named = face.personId != null && !!face.name;
  return tidy([
    { label: named ? "Tag as someone else…" : "Tag…", icon: icon(IconTag), onSelect: a.tag },
    { label: "New person…", icon: icon(IconUserPlus), onSelect: a.newPerson },
    named && a.useAsMain && { label: `Use as main photo of ${face.name}`, icon: icon(IconUserCircle), onSelect: a.useAsMain },
    DIVIDER,
    named && a.untag && { label: `Not ${face.name}`, icon: icon(IconX), onSelect: a.untag },
    a.notAFace && { label: "Not a face", icon: icon(IconMoodEmpty), onSelect: a.notAFace },
  ]);
}

export interface PetMenuActions {
  open: () => void;
  /** Start choosing several pets with this one picked (only where the page has a select mode). */
  select?: () => void;
  rename: () => void;
  merge?: () => void;
  /** Merging needs another pet of the same species. */
  canMerge?: boolean;
  notAPet: () => void;
  remove: () => void;
}

/** A pet card on `/pets` (Round 8B): Open · Select · Rename · Merge with… · — · Not a pet · Remove from Pets. */
export function petMenu(p: PetSummary, a: PetMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    a.select && { label: "Select", icon: icon(IconCircleCheck), shortcut: "Ctrl+Click", onSelect: a.select },
    { label: p.name ? "Rename" : "Add a name", icon: icon(IconPencil), onSelect: a.rename },
    a.merge && { label: "Merge with…", icon: icon(IconArrowsJoin), disabled: a.canMerge === false, onSelect: a.merge },
    DIVIDER,
    { label: "Not a pet", icon: icon(IconMoodEmpty), onSelect: a.notAPet },
    { label: "Remove from Pets", icon: icon(IconPawOff), danger: true, onSelect: a.remove },
  ]);
}

export interface PetBoxMenuActions {
  /** *Open {name}*: the info panel's rows only. */
  open?: () => void;
  tag: () => void;
  newPet: () => void;
  /** *Use as main photo of {name}*: only for a box given to a pet. */
  useAsMain?: () => void;
  /** *Not {name}*: only for a box given to a pet. */
  untag?: () => void;
  notAPet: () => void;
}

/**
 * A pet box in the viewer (*Show pets*) or a pet row of the info panel (Round 8B): Open {name} ·
 * Tag as… · New pet… · Use as main photo of {name} · — · Not {name} · Not a pet.
 */
export function petBoxMenu(box: AssetPet, a: PetBoxMenuActions): MenuItem[] {
  const named = box.petId != null && !!box.name;
  return tidy([
    named && a.open && { label: `Open ${box.name}`, icon: icon(IconExternalLink), onSelect: a.open },
    { label: named ? `Tag as another ${box.species}…` : "Tag as…", icon: icon(IconTag), onSelect: a.tag },
    { label: "New pet…", icon: icon(IconPaw), onSelect: a.newPet },
    named && a.useAsMain && { label: `Use as main photo of ${box.name}`, icon: icon(IconPhotoStar), onSelect: a.useAsMain },
    DIVIDER,
    named && a.untag && { label: `Not ${box.name}`, icon: icon(IconX), onSelect: a.untag },
    { label: "Not a pet", icon: icon(IconMoodEmpty), onSelect: a.notAPet },
  ]);
}

export interface SlideshowMenuActions {
  open: () => void;
  /** Play and Export belong to Phase F's editor; they appear when it hands them over. */
  play?: () => void;
  export?: () => void;
  duplicate: () => void;
  rename: () => void;
  delete: () => void;
}

/** A slideshow row: Open · Play · Export… · Duplicate · Rename · — · Delete (undo). */
export function slideshowMenu(a: SlideshowMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    a.play && { label: "Play", icon: icon(IconPlayerPlay), onSelect: a.play },
    a.export && { label: "Export…", icon: icon(IconDownload), onSelect: a.export },
    { label: "Duplicate", icon: icon(IconCopy), onSelect: a.duplicate },
    { label: "Rename…", icon: icon(IconPencil), onSelect: a.rename },
    DIVIDER,
    { label: "Delete", icon: icon(IconTrash), danger: true, onSelect: a.delete },
  ]);
}

export interface ImportRowMenuActions {
  /** The folder it came from (read-only); only for a folder import whose path is known. */
  showInFolder?: () => void;
  /** Folder imports with failures only. */
  retryFailed?: () => void;
  /** Rows with failures only. */
  copyErrors?: () => void;
  dismiss: () => void;
}

/** A row in *Recent imports*: Show in folder · Retry failed · Copy error text · — · Dismiss. */
export function importRowMenu(a: ImportRowMenuActions): MenuItem[] {
  return tidy([
    a.showInFolder && { label: "Show in folder", icon: icon(IconFolderOpen), onSelect: a.showInFolder },
    a.retryFailed && { label: "Retry failed", icon: icon(IconRefresh), onSelect: a.retryFailed },
    a.copyErrors && { label: "Copy error text", icon: icon(IconClipboard), onSelect: a.copyErrors },
    DIVIDER,
    { label: "Dismiss", icon: icon(IconX), onSelect: a.dismiss },
  ]);
}

export interface SetMenuActions {
  open: () => void;
  /** The viewer playing through the set, four seconds a photo. */
  slideshow?: () => void;
  export?: () => void;
  /** Library and Favorites: the set in Develop's filmstrip. */
  develop?: () => void;
}

/**
 * A set of photos with no menu of its own: a collection card (Open · Start slideshow ·
 * Export…) or a sidebar entry (Open · Open in Develop · Start slideshow).
 */
export function setMenu(a: SetMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    a.develop && { label: "Open in Develop", icon: icon(IconAperture), onSelect: a.develop },
    a.slideshow && { label: "Start slideshow", icon: icon(IconPlayerPlay), onSelect: a.slideshow },
    a.export && { label: "Export…", icon: icon(IconDownload), onSelect: a.export },
  ]);
}
