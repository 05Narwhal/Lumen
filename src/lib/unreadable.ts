import { invoke } from "@/lib/ipc";
import type { UnreadablePage } from "@/types/generated/UnreadablePage";

/** `assets=` value meaning every file Lumen can't open (*Restore all…*). */
export const ALL_UNREADABLE = "unreadable";

/** The ids in an `assets=` value: a list of numbers, or [`ALL_UNREADABLE`] (resolved from the list). */
export async function restoreIds(param: string): Promise<number[]> {
  if (param !== ALL_UNREADABLE) return param.split(",").map(Number).filter((n) => Number.isInteger(n) && n > 0);
  return allUnreadableIds();
}

/** Every file Lumen can't open (not in the Trash), paging the list. */
export async function allUnreadableIds(): Promise<number[]> {
  const ids: number[] = [];
  let after: number | null = null;
  for (;;) {
    const page: UnreadablePage = await invoke<UnreadablePage>("list_unreadable", { after, limit: 500 });
    ids.push(...page.items.map((i) => i.id));
    if (page.nextCursor == null) return ids;
    after = page.nextCursor;
  }
}
