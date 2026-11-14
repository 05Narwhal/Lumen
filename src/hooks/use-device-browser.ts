import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { DEFAULT_FILTER, extendSelection, filterKey, invertWithin, stillPresent } from "@/components/device-browser/model";
import { DEVICE_BROWSER_CHANGED, DEVICE_IPHONES, DEVICE_MACDISKS, DEVICE_VOLUMES } from "@/lib/events";
import { checkRoom } from "@/lib/import-space";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { BrowseTarget } from "@/types/generated/BrowseTarget";
import type { BrowserChanged } from "@/types/generated/BrowserChanged";
import type { BrowserFilter } from "@/types/generated/BrowserFilter";
import type { BrowserPage } from "@/types/generated/BrowserPage";
import type { BrowserSelection } from "@/types/generated/BrowserSelection";
import type { BrowserSession } from "@/types/generated/BrowserSession";

const PAGE = 200;
const NONE: BrowserSelection = { count: 0, files: 0, bytes: 0, imported: 0 };

/**
 * One device browser session (Round 4E): opened on mount, closed on unmount. Items arrive in
 * pages under the filter; the selection is kept here by item index, so it survives filter
 * changes and the re-sort when a card's own dates have been read. `device-browser://changed`
 * refetches what is loaded; a watcher reporting the device gone flips `gone`.
 */
export function useDeviceBrowser(target: BrowseTarget) {
  const qc = useQueryClient();
  const targetKey = JSON.stringify(target);
  // Each browser is its own session, opened here and closed when this unmounts; a session
  // that arrives after its effect was cleaned up (StrictMode, a fast close) is closed at once.
  const [opened, setOpened] = useState<{ data: BrowserSession | null; error: unknown; loading: boolean }>({ data: null, error: null, loading: true });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let alive = true;
    let sid: number | null = null;
    setOpened({ data: null, error: null, loading: true });
    invoke<BrowserSession>("open_device_browser", { target: JSON.parse(targetKey) as BrowseTarget }).then(
      (s) => {
        if (alive) {
          sid = s.id;
          setOpened({ data: s, error: null, loading: false });
        } else {
          void invoke("close_device_browser", { session: s.id }).catch(() => {});
        }
      },
      (error: unknown) => alive && setOpened({ data: null, error, loading: false }),
    );
    return () => {
      alive = false;
      if (sid !== null) void invoke("close_device_browser", { session: sid }).catch(() => {});
    };
  }, [targetKey, attempt]);
  const retry = useCallback(() => setAttempt((n) => n + 1), []);
  const id = opened.data?.id ?? null;

  const summary = useQuery({
    queryKey: queryKeys.deviceBrowser.summary(id ?? 0),
    queryFn: () => invoke<BrowserSession>("get_device_browser", { session: id }),
    enabled: id !== null,
    initialData: opened.data ?? undefined,
    staleTime: Infinity,
  });

  const [filter, setFilter] = useState<BrowserFilter>(DEFAULT_FILTER);
  const fk = filterKey(filter);

  const pages = useInfiniteQuery({
    queryKey: queryKeys.deviceBrowser.pages(id ?? 0, fk),
    queryFn: ({ pageParam }) => invoke<BrowserPage>("browse_device", { session: id, filter, cursor: pageParam, limit: PAGE }),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.nextCursor,
    enabled: id !== null,
    staleTime: Infinity,
    placeholderData: (prev) => prev,
  });
  const items = useMemo(() => pages.data?.pages.flatMap((p) => p.items) ?? [], [pages.data]);
  const matching = pages.data?.pages[0]?.matching ?? 0;

  // Every index under the filter: *Select all*, *Invert*, and what the selection hides.
  const shown = useQuery({
    queryKey: queryKeys.deviceBrowser.ids(id ?? 0, fk),
    queryFn: () => invoke<number[]>("device_browser_ids", { session: id, filter }),
    enabled: id !== null,
    staleTime: Infinity,
    placeholderData: (prev) => prev,
  });
  const shownSet = useMemo(() => new Set(shown.data ?? []), [shown.data]);

  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [anchor, setAnchor] = useState<number | null>(null);
  const [version, setVersion] = useState(0);
  const select = useCallback((next: ReadonlySet<number>, newAnchor?: number | null) => {
    setSelected(next);
    if (newAnchor !== undefined) setAnchor(newAnchor);
    setVersion((v) => v + 1);
  }, []);
  const latest = useRef({ selected, anchor, items, shown: shown.data ?? [] });
  latest.current = { selected, anchor, items, shown: shown.data ?? [] };

  const selection = useQuery({
    queryKey: queryKeys.deviceBrowser.selection(id ?? 0, version),
    queryFn: () => invoke<BrowserSelection>("device_browser_selection", { session: id, indexes: [...latest.current.selected] }),
    enabled: id !== null && selected.size > 0,
    staleTime: Infinity,
    placeholderData: (prev) => prev,
  });

  const [gone, setGone] = useState(false);
  useEffect(() => {
    const offs: Promise<() => void>[] = [];
    offs.push(
      listen<BrowserChanged>(DEVICE_BROWSER_CHANGED, (e) => {
        if (e.payload.session !== id) return;
        void qc.invalidateQueries({ queryKey: queryKeys.deviceBrowser.session(e.payload.session) });
      }),
    );
    const watch = (name: string, kind: "volumes" | "iphones" | "macdisks") =>
      listen(name, (e) => {
        if (stillPresent(target, kind, e.payload) === false) setGone(true);
      });
    offs.push(watch(DEVICE_VOLUMES, "volumes"), watch(DEVICE_IPHONES, "iphones"), watch(DEVICE_MACDISKS, "macdisks"));
    return () => offs.forEach((p) => void p.then((off) => off()));
  }, [id, qc, targetKey]);

  const order = useMemo(() => items.map((i) => i.index), [items]);

  /** A click on a tile: alone, with Shift (a range from the last click) or Ctrl/Cmd (toggle). */
  const click = useCallback(
    (index: number, mods: { shift: boolean; toggle: boolean }) => {
      const { selected: sel, anchor: a } = latest.current;
      if (mods.shift) return select(extendSelection(sel, order, a, index));
      const next = new Set(sel);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      // A plain click toggles too: picking items to import is building up a set, not
      // choosing one photo, so nothing already picked is lost to a stray click.
      select(next, index);
    },
    [order, select],
  );

  const selectAll = useCallback(() => select(new Set([...latest.current.selected, ...latest.current.shown]), null), [select]);
  const selectNone = useCallback(() => select(new Set(), null), [select]);
  const invert = useCallback(() => select(invertWithin(latest.current.selected, latest.current.shown), null), [select]);
  const selectMany = useCallback((ids: number[], add: boolean) => {
    const next = new Set(latest.current.selected);
    for (const i of ids) {
      if (add) next.add(i);
      else next.delete(i);
    }
    select(next);
  }, [select]);

  const hiddenSelected = useMemo(() => {
    if (!shown.data) return 0;
    let n = 0;
    for (const i of selected) if (!shownSet.has(i)) n++;
    return n;
  }, [selected, shown.data, shownSet]);

  /** Space check first, as every import does; resolves to the batch id, or null if not started. */
  const startImport = useCallback(async (): Promise<number | null> => {
    if (id === null) return null;
    const indexes = [...latest.current.selected];
    const room = await checkRoom({ kind: "browser", session: id, indexes });
    if (room === false) return null;
    const batch = await invoke<number>("start_device_browser_import", { session: id, indexes, limitBytes: room });
    void qc.invalidateQueries({ queryKey: queryKeys.batches });
    return batch;
  }, [id, qc]);

  return {
    session: summary.data ?? opened.data ?? null,
    error: opened.error,
    loading: opened.loading,
    retry,
    gone,
    filter,
    setFilter,
    items,
    matching,
    hasMore: pages.hasNextPage,
    loadMore: pages.fetchNextPage,
    loadingMore: pages.isFetchingNextPage,
    selected,
    selection: selected.size === 0 ? NONE : (selection.data ?? { ...NONE, count: selected.size }),
    hiddenSelected,
    click,
    selectAll,
    selectNone,
    invert,
    selectMany,
    startImport,
  };
}
