import { IconCopy, IconDeviceFloppy, IconFileImport, IconFolderOpen } from "@tabler/icons-react";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export interface PresetsMenuContext {
  /** Camera Raw's settings folder, when Lightroom or Camera Raw is installed here. */
  lightroomFolder: string | null;
}

export interface PresetsMenuActions {
  save: () => void;
  importFiles: () => void;
  importFolder: () => void;
  importFromLightroom: () => void;
}

/** The presets list's ⋯ menu (Round 11): Save preset… · — · the Lightroom imports. */
export function presetsMenu(ctx: PresetsMenuContext, a: PresetsMenuActions): MenuItem[] {
  return tidy([
    { label: "Save preset…", icon: icon(IconDeviceFloppy), onSelect: a.save },
    DIVIDER,
    { label: "Import Lightroom presets…", icon: icon(IconFileImport), onSelect: a.importFiles },
    { label: "Import a folder of Lightroom presets…", icon: icon(IconFolderOpen), onSelect: a.importFolder },
    ctx.lightroomFolder !== null && { label: "Import from Lightroom", icon: icon(IconFileImport), onSelect: a.importFromLightroom },
  ]);
}

/** A plugin preset's right-click menu: it is read-only, so it can only be copied to *Your presets*. */
export function pluginPresetMenu(a: { duplicate: () => void }): MenuItem[] {
  return [{ label: "Duplicate to my presets", icon: icon(IconCopy), onSelect: a.duplicate }];
}
