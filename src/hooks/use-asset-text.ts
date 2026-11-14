import { useQuery } from "@tanstack/react-query";
import { useSettings } from "@/hooks/use-settings";
import { useEditVersion } from "@/lib/edit-versions";
import { invoke } from "@/lib/ipc";
import { readTextOn } from "@/lib/photo-text";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";

const NONE: ShownTextLine[] = [];

/**
 * The text read in one photo, on the photo as shown (Round 13-3). `asset_text` reads it
 * first when it has not been read yet, so `reading` is true meanwhile. The edit version is
 * in the key: a new crop or turn moves the lines. Nothing while text is off or for a video.
 */
export function useAssetText(id: number | null, still: boolean): { lines: ShownTextLine[]; reading: boolean; on: boolean } {
  const { settings } = useSettings();
  const on = readTextOn(settings);
  const version = useEditVersion(id ?? -1);
  const enabled = on && still && id !== null;
  const q = useQuery({
    queryKey: ["ocr", "text", id, version ?? null],
    queryFn: () => invoke<ShownTextLine[]>("asset_text", { id }),
    enabled,
  });
  return { lines: enabled ? (q.data ?? NONE) : NONE, reading: enabled && q.isPending, on };
}
