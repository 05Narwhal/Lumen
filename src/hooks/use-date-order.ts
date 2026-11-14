import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { useSettings } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import type { DateOrder } from "@/types/generated/DateOrder";
import type { DateOrderSetting } from "@/types/generated/DateOrderSetting";
import { setDateOrder } from "@/utils/date";

export const systemDateOrderQuery = { queryKey: ["system-date-order"] as const, queryFn: () => invoke<DateOrder>("system_date_order"), staleTime: Infinity };

/** The order a setting means, given what *Automatic* is on this system. */
export const resolveDateOrder = (setting: DateOrderSetting | undefined, system: DateOrder | undefined): DateOrder =>
  !setting || setting === "auto" ? (system ?? "dmy") : setting;

/**
 * The date order in force (Round 11), handed to `utils/date` so every date shown follows it.
 * Mounted once at the app's root; Rust resolves the same order for search.
 */
export function useDateOrder(): { order: DateOrder; system: DateOrder | undefined; setting: DateOrderSetting } {
  const { settings } = useSettings();
  const system = useQuery(systemDateOrderQuery).data;
  const setting = settings?.search?.dateOrder ?? "auto";
  const order = resolveDateOrder(setting, system);
  useEffect(() => setDateOrder(order), [order]);
  return { order, system, setting };
}
