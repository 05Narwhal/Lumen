import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { Settings } from "@/types/generated/Settings";

export type SettingsPatch = (s: Settings) => Settings;

export function useSettings() {
  const qc = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.settings,
    queryFn: () => invoke<Settings>("get_settings"),
    staleTime: Infinity,
  });
  const mutation = useMutation({
    mutationFn: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
    onMutate: async (next) => {
      await qc.cancelQueries({ queryKey: queryKeys.settings });
      const prev = qc.getQueryData<Settings>(queryKeys.settings);
      qc.setQueryData(queryKeys.settings, next);
      return { prev };
    },
    onError: (_e, _v, ctx) => {
      if (ctx?.prev) qc.setQueryData(queryKeys.settings, ctx.prev);
    },
    onSuccess: (saved, _v, ctx) => {
      qc.setQueryData(queryKeys.settings, saved);
      // Facet's switch (8f) shows or hides the Masks panel's AI items; turning it on fetches the model.
      if (!!ctx?.prev?.smart?.experimental?.aiMasks !== !!saved?.smart?.experimental?.aiMasks) {
        void qc.invalidateQueries({ queryKey: ["ai-masks"] });
        void qc.invalidateQueries({ queryKey: queryKeys.experimentalCatalog });
      }
    },
  });
  const { mutate } = mutation;
  const update = useCallback(
    (patch: SettingsPatch) => {
      const cur = qc.getQueryData<Settings>(queryKeys.settings);
      if (cur) mutate(patch(cur));
    },
    [qc, mutate],
  );
  return { settings: query.data, isLoading: query.isLoading, update };
}

/**
 * A control's own value, changed the instant it is clicked and reconciled with the stored
 * value when the round trip lands.
 *
 * Settings persist through `invoke`, so react-query applies the optimistic cache write in
 * a microtask rather than inside the click handler. React therefore cannot flush it as a
 * discrete update: the DOM change arrives about two frames later, by which point the
 * browser has already recomputed the element's style, so no CSS transition is created and
 * the control jumps to its new position. Measured in the real webview: a segmented
 * control driven by local state reports two running transitions synchronously on click,
 * the same control driven by settings reports none.
 *
 * Holding the value locally puts the visual change back inside the click, where a
 * transition can start; persistence catches up behind it.
 */
export function useSettingValue<T>(stored: T): [T, (v: T) => void] {
  const [local, setLocal] = useState(stored);
  useEffect(() => setLocal(stored), [stored]);
  return [local, setLocal];
}
