import type React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import { ratingOverrides } from "@/hooks/use-ratings";
import { PhotoTile, type TileModifiers } from "@/components/photo-tile";
import { playStarBurst } from "@/components/star-burst";
import { favoriteOverrides, useFavorites } from "@/hooks/use-favorites";
import { useTileActions } from "@/hooks/use-tile-actions";
import { useElementSize } from "@/hooks/use-element-size";
import { useLayout, useLayoutRows } from "@/hooks/use-layout";
import { selectAll, selectRange, selectionStore, toggleSelection, useSelection } from "@/hooks/use-selection";
import { invoke } from "@/lib/ipc";
import { matches } from "@/lib/keybinds";
import type { GroupBy } from "@/types/generated/GroupBy";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { Section } from "@/types/generated/Section";
import { autoScrollStep, idsInMarquee, neighbourId, normalizeRect, type PlacedRow, type Rect } from "./marquee";
import { clearViewerScope, setViewerScope, viewerScopeStore } from "@/lib/viewer-scope";
import { gridOnScreen, noteRadarGrid } from "@/lib/radar-view";
import { TimelineScrubber } from "@/components/timeline-scrubber";
import "./styles.scss";

/** Drag payload: JSON array of asset ids, dropped on sidebar albums. */
export const DRAG_MIME = "application/x-lumen-asset-ids";

export interface PhotoGridProps {
  rowHeight: number;
  scope: LayoutScope;
  /** Day, month or year sections for a time-ordered scope; month when not given. */
  groupBy?: GroupBy;
  /** Scroll to this photo once its layout is known, then report it with `onFocused`. */
  focusAsset?: number | null;
  onFocused?: () => void;
  /** `section` is the section the tile was clicked in: only a collection's photos by album repeat a photo. */
  onOpen: (id: number, section?: number) => void;
  /** A collection's photos by album: a section header opens its album. */
  onOpenSection?: (albumId: number) => void;
  /** Extra context-menu items for the surface hosting the grid (e.g. "Remove from album"). */
  extraMenuItems?: (ids: number[]) => MenuItem[];
  /** Shown in the grid's place when the scope holds nothing (e.g. *Has text*, Round 13-3). */
  empty?: React.ReactNode;
  /**
   * Pick mode (Round 19 §5): a click only toggles the tile; no viewer, menu, drag or favorite.
   * `locked` photos (already in the album) show checked and cannot be toggled.
   */
  pick?: { locked: ReadonlySet<number> };
}

/** Reads the theme's tile gap once so Rust and CSS agree on it. */
function readGap(): number {
  if (typeof window === "undefined") return 4;
  const v = getComputedStyle(document.documentElement).getPropertyValue("--tile-gap").trim();
  const n = parseFloat(v);
  return Number.isFinite(n) ? n : 4;
}

function sectionAt(sections: Section[], rowIndex: number): Section | undefined {
  let lo = 0;
  let hi = sections.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const s = sections[mid];
    if (rowIndex < s.firstRow) hi = mid - 1;
    else if (rowIndex >= s.firstRow + s.rowCount) lo = mid + 1;
    else return s;
  }
  return undefined;
}

const NO_ROWS: number[] = [];

interface GridRowProps {
  row: LayoutRow;
  top: number;
  selected: ReadonlySet<number>;
  favorites: ReadonlyMap<number, boolean>;
  onOpen: (id: number, section?: number) => void;
  onToggle: (id: number, mods: TileModifiers) => void;
  onFavorite?: (id: number) => void;
  onContextMenu?: (id: number, e: React.MouseEvent) => void;
  onDragStart?: (id: number, e: React.DragEvent<HTMLButtonElement>) => void;
  /** Pick mode: the photos that are already in the album. */
  locked?: ReadonlySet<number>;
  compact: boolean;
  /** Tiles that have not loaded yet wait (see `flying`). */
  hold: boolean;
}

/** Faster than this (px per ms) the Years view holds new images; they load once it settles. */
const FLYING_PX_PER_MS = 1.2;
const SETTLE_MS = 120;

/**
 * One row of tiles. Memoised: while scrolling, only rows entering the view render; the
 * rest are skipped whole instead of each tile comparing its props every frame.
 */
const GridRow = memo(function GridRow({ row, top, selected, favorites, onOpen, onToggle, onFavorite, onContextMenu, onDragStart, locked, compact, hold }: GridRowProps) {
  const section = row.section;
  const openHere = useCallback((id: number) => onOpen(id, section), [onOpen, section]);
  return (
    <div className="photo-grid__row" style={{ top, height: row.height }} role="row">
      {row.tiles.map((t) => (
        <PhotoTile
          key={t.id}
          tile={t}
          top={0}
          selected={selected.has(t.id) || !!locked?.has(t.id)}
          selecting={locked ? true : selected.size > 0}
          pick={locked ? (locked.has(t.id) ? "locked" : "free") : undefined}
          favorite={favorites.get(t.id)}
          onOpen={openHere}
          onToggle={onToggle}
          onFavorite={onFavorite}
          onContextMenu={onContextMenu}
          onDragStart={onDragStart}
          compact={compact}
          hold={hold}
        />
      ))}
    </div>
  );
});

export function PhotoGrid({ rowHeight, scope, groupBy = "month", focusAsset = null, onFocused, onOpen: openFromRoute, onOpenSection, extraMenuItems, empty, pick }: PhotoGridProps) {
  const { setFavorite } = useFavorites();
  const overrides = favoriteOverrides.use().map;
  const { open: openMenu } = useContextMenu();
  const { menuItemsFor } = useTileActions();
  const ref = useRef<HTMLDivElement>(null);
  // Measure the inner track, not the scroll element: padding and the scrollbar gutter
  // must not be part of the width Rust lays rows out for.
  const [innerRef, size] = useElementSize<HTMLDivElement>(80);
  const [gap, setGap] = useState(readGap);
  useEffect(() => setGap(readGap()), [rowHeight]);
  const params = useMemo(
    () => ({ width: Math.max(0, size.width), rowHeight, gap, scope, groupBy }),
    [size.width, rowHeight, gap, scope, groupBy],
  );
  // The viewer pages through this grid, in its order, so it is told which grid opened it.
  const paramsRef = useRef(params);
  paramsRef.current = params;
  const onOpen = useCallback(
    (id: number, section?: number) => {
      setViewerScope(paramsRef.current);
      openFromRoute(id, section);
    },
    [openFromRoute],
  );
  // The face radar checks what this grid shows (Round 10).
  useEffect(() => {
    noteRadarGrid(params);
    return () => {
      if (gridOnScreen() === params) noteRadarGrid(null);
    };
  }, [params]);
  // A grid that is gone is no scope to page through: a viewer opened later from a screen
  // without a grid falls back to the timeline instead of a stale album.
  useEffect(
    () => () => {
      const shown = viewerScopeStore.get();
      if (shown && JSON.stringify(shown.scope) === JSON.stringify(paramsRef.current.scope)) clearViewerScope();
    },
    [],
  );
  // Brief fade while rows for a new width load, instead of stale rows overlapping.
  const [resizing, setResizing] = useState(false);
  useEffect(() => {
    setResizing(true);
    const t = setTimeout(() => setResizing(false), 220);
    return () => clearTimeout(t);
  }, [size.width, rowHeight, groupBy]);
  const { summary, fingerprint, isPlaceholder } = useLayout(params);
  const rowHeights = summary?.rowHeights ?? NO_ROWS;
  const sections = summary?.sections ?? [];
  // Only a titled section starts with a header row. An unsectioned scope (an album in manual
  // order, a memory, a person) has one untitled section whose first row is photos; counting
  // it as a header gave that row no gap, so the first two rows touched.
  const headerRows = useMemo(() => new Set(sections.filter((s) => s.title !== "").map((s) => s.firstRow)), [sections]);

  // Dragging the scrubber passes whole screens a frame in any view: hold every new tile's
  // image until the drag ends, as flying through the Years squares does.
  const [scrubbing, setScrubbing] = useState(false);
  const virtualizer = useVirtualizer({
    count: rowHeights.length,
    getScrollElement: () => ref.current,
    estimateSize: (i) => rowHeights[i] + (headerRows.has(i) ? 0 : gap),
    // Rows, not pixels: twelve 56 px rows of Years squares mounted ~300 tiles past each edge.
    // A scrubber drag lands on a new screen every frame, so rows past the edge are never
    // scrolled into; building them was most of the frame's layout (Round 4H, measured).
    overscan: scrubbing ? 2 : groupBy === "year" ? 4 : 12,
    initialRect: { width: 1280, height: 800 },
  });
  // The virtualizer caches row offsets and only re-reads `estimateSize` when the count
  // changes; when heights change at the same count, rows kept stale offsets and gaps.
  useLayoutEffect(() => {
    virtualizer.measure();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rowHeights, gap]);
  const virtualRows = virtualizer.getVirtualItems();
  const range: [number, number] | null = virtualRows.length
    ? [virtualRows[0].index, virtualRows[virtualRows.length - 1].index]
    : null;
  const rows = useLayoutRows(params, range, fingerprint);

  const selection = useSelection();
  const pickLocked = pick?.locked;
  const orderRef = useRef<number[]>([]);
  useEffect(() => {
    const ids: number[] = [];
    for (const i of [...rows.keys()].sort((a, b) => a - b)) for (const t of rows.get(i)!.tiles) ids.push(t.id);
    orderRef.current = ids;
  }, [rows]);

  // Every callback a tile gets is created once and reads what changes through `latest`: a
  // new function on any render re-rendered every tile on screen (a thousand in Years) on
  // every scroll frame.
  const latest = useRef({ rows, overrides, setFavorite, openMenu, menuItemsFor, onOpen, extraMenuItems });
  latest.current = { rows, overrides, setFavorite, openMenu, menuItemsFor, onOpen, extraMenuItems };
  const tileById = useCallback((id: number) => {
    for (const r of latest.current.rows.values()) for (const t of r.tiles) if (t.id === id) return t;
    return undefined;
  }, []);
  const onToggle = useCallback((id: number, mods: TileModifiers) => {
    if (mods.shift) selectRange(id, orderRef.current, paramsRef.current);
    else toggleSelection(id);
  }, []);
  const onFavorite = useCallback(
    (id: number) => {
      const { overrides, setFavorite } = latest.current;
      const current = overrides.get(id) ?? tileById(id)?.isFavorite ?? false;
      if (!current) playStarBurst(document.getElementById(`tile-${id}`));
      void setFavorite([id], !current);
    },
    [tileById],
  );
  const onTileDrag = useCallback(
    (id: number, e: React.DragEvent<HTMLButtonElement>) => {
      const sel = selectionStore.get().ids;
      const ids = sel.has(id) ? [...sel] : [id];
      e.dataTransfer.setData(DRAG_MIME, JSON.stringify(ids));
      e.dataTransfer.effectAllowed = "copy";
    },
    [],
  );
  const onTileMenu = useCallback(
    (id: number, e: React.MouseEvent) => {
      const { overrides, openMenu, menuItemsFor, onOpen, extraMenuItems } = latest.current;
      const sel = selectionStore.get().ids;
      const ids = sel.has(id) ? [...sel] : [id];
      const fav = overrides.get(id) ?? tileById(id)?.isFavorite ?? false;
      const rating = ids.length === 1 ? (ratingOverrides.get().map.get(id) ?? tileById(id)?.rating ?? 0) : 0;
      const media = ids.flatMap((i) => {
        const t = tileById(i);
        return t ? [{ mediaType: t.mediaType, raw: t.raw }] : [];
      });
      openMenu(e, [...menuItemsFor(ids, { favorite: fav, rating, onOpen, media: media.length === ids.length ? media : undefined }), ...(extraMenuItems?.(ids) ?? [])]);
    },
    [tileById],
  );

  // Pin the section title only once its own header row has scrolled out of view.
  const firstTileRow = virtualRows.find((v) => !headerRows.has(v.index));
  const scrollTop = virtualizer.scrollOffset ?? 0;
  const pinnedCandidate = firstTileRow ? sectionAt(sections, firstTileRow.index) : undefined;
  const pinned = pinnedCandidate && pinnedCandidate.title && scrollTop > pinnedCandidate.y + 8 ? pinnedCandidate : undefined;
  // The scrubber names dates only for a grid grouped by date: an album in manual order, a
  // memory or a person has one untitled section.
  const scrubberGrouping = sections.some((s) => s.title !== "") ? groupBy : null;

  // Flying through the Years squares: new tiles hold their images (their colour shows) and
  // load the moment the view slows. Each image is a request into Rust, and a fast scroll
  // mounts a row of ~25 a frame; asking for all of them is what dropped frames.
  const [flying, setFlying] = useState(false);
  const compact = groupBy === "year";
  useEffect(() => {
    const el = ref.current;
    if (!el || !compact) return;
    let lastTop = el.scrollTop;
    let lastT = performance.now();
    let settle: ReturnType<typeof setTimeout>;
    const on = () => {
      const now = performance.now();
      const speed = Math.abs(el.scrollTop - lastTop) / Math.max(1, now - lastT);
      lastTop = el.scrollTop;
      lastT = now;
      if (speed > FLYING_PX_PER_MS) setFlying(true);
      clearTimeout(settle);
      settle = setTimeout(() => setFlying(false), SETTLE_MS);
    };
    el.addEventListener("scroll", on, { passive: true });
    return () => {
      clearTimeout(settle);
      el.removeEventListener("scroll", on);
      setFlying(false);
    };
  }, [ref, compact]);

  // Rows in canvas coordinates, for the marquee and arrow keys.
  const placed: PlacedRow[] = useMemo(
    () => virtualRows.flatMap((v) => (rows.get(v.index) ? [{ row: rows.get(v.index) as LayoutRow, top: v.start }] : [])),
    [virtualRows, rows],
  );

  const [band, setBand] = useState<Rect | null>(null);
  const drag = useRef<{ x: number; y: number; base: Set<number>; mode: "replace" | "add" | "toggle" } | null>(null);
  const [focused, setFocused] = useState<number | null>(null);

  // Opening on a photo: the row comes from Rust, because the rows around it are not loaded.
  // Waits for this layout's own summary, so the index is scrolled in the right geometry.
  const summaryReady = summary !== undefined && !isPlaceholder && params.width > 0;
  useEffect(() => {
    if (focusAsset == null || !summaryReady) return;
    let live = true;
    void invoke<number | null>("find_layout_row", { ...params, assetId: focusAsset }).then((row) => {
      if (!live) return;
      if (row != null) {
        virtualizer.scrollToIndex(row, { align: "center" });
        setFocused(focusAsset);
      }
      onFocused?.();
    });
    return () => {
      live = false;
    };
    // `virtualizer` is a new object every render; the request depends on the layout alone.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusAsset, summaryReady, params, onFocused]);

  // A drag that starts on the background rubber-bands; one that starts on a tile is the
  // tile's own (drag to an album), so tiles stop the event before it reaches here.
  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const el = ref.current;
      if (!el || e.button !== 0) return;
      if ((e.target as HTMLElement).closest(".photo-tile, .photo-grid__title-link")) return;
      const box = el.getBoundingClientRect();
      const x = e.clientX - box.left;
      const y = e.clientY - box.top + el.scrollTop;
      const mode = e.shiftKey ? "add" : e.ctrlKey || e.metaKey ? "toggle" : "replace";
      drag.current = { x, y, base: new Set(mode === "replace" ? [] : selection.ids), mode };
      if (mode === "replace") selection.clear();
      setBand({ x, y, w: 0, h: 0 });
      el.setPointerCapture(e.pointerId);
    },
    [selection],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const el = ref.current;
      const d = drag.current;
      if (!el || !d) return;
      const box = el.getBoundingClientRect();
      el.scrollTop += autoScrollStep(e.clientY - box.top, box.height);
      const next = normalizeRect(d.x, d.y, e.clientX - box.left, e.clientY - box.top + el.scrollTop);
      setBand(next);
      const hit = idsInMarquee(placed, next);
      const ids = new Set(d.base);
      for (const id of hit) {
        if (d.mode === "toggle" && d.base.has(id)) ids.delete(id);
        else ids.add(id);
      }
      selectAll([...ids]);
    },
    [placed],
  );

  const endDrag = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    drag.current = null;
    setBand(null);
    ref.current?.releasePointerCapture(e.pointerId);
  }, []);

  // Roving focus: the grid owns one focused tile and the arrows move it.
  const onGridKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const dirs: Record<string, "up" | "down" | "left" | "right"> = {
        ArrowUp: "up",
        ArrowDown: "down",
        ArrowLeft: "left",
        ArrowRight: "right",
      };
      const dir = dirs[e.key];
      const current = focused ?? orderRef.current[0] ?? null;
      if (dir) {
        if (current === null) return;
        e.preventDefault();
        const next = focused === null ? current : neighbourId(placed, current, dir);
        if (next === null) return;
        setFocused(next);
        if (e.shiftKey) selection.range(next, orderRef.current);
        document.getElementById(`tile-${next}`)?.querySelector("button")?.focus();
        return;
      }
      if (current === null) return;
      if (matches(e.nativeEvent, "grid.open")) {
        e.preventDefault();
        // Pick mode: Enter adds the picked photos (the page listens), it does not open a photo.
        if (!pickLocked) onOpen(current);
      }
    },
    [focused, placed, selection, onOpen, pickLocked],
  );

  return (
    <div className="photo-grid-frame">
    <div
      className="photo-grid"
      data-part="grid"
      ref={ref}
      role="grid"
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onGridKeyDown}
      aria-activedescendant={focused !== null ? `tile-${focused}` : undefined}
      aria-rowcount={rowHeights.length}
      data-selecting={selection.count > 0}
      data-resizing={resizing}
    >
      <div className="photo-grid__topfade" data-visible={scrollTop > 4} aria-hidden />
      {pinned && (
        <div className="photo-grid__pinned" aria-hidden>
          {pinned.title}
        </div>
      )}
      <div className="photo-grid__inner" ref={innerRef}>
      <div className="photo-grid__canvas" style={{ height: virtualizer.getTotalSize() }}>
        {band && band.w > 2 && band.h > 2 && (
          <div className="photo-grid__marquee" style={{ left: band.x, top: band.y, width: band.w, height: band.h }} aria-hidden />
        )}
        {virtualRows.map((v) => {
          const row: LayoutRow | undefined = rows.get(v.index);
          // A row that does not fit this layout is from another arrangement: never draw it.
          if (!row || row.height !== rowHeights[v.index]) return null;
          if (row.kind === "header") {
            const album = onOpenSection ? sections[row.section]?.albumId : null;
            return (
              <div key={v.key} className="photo-grid__header" style={{ top: v.start, height: row.height }} role="row">
                {album != null ? (
                  <h2 className="photo-grid__title">
                    <button type="button" className="photo-grid__title-link" onClick={() => onOpenSection?.(album)}>
                      {row.title}
                    </button>
                  </h2>
                ) : (
                  <h2 className="photo-grid__title">{row.title}</h2>
                )}
              </div>
            );
          }
          return (
            <GridRow
              key={v.key}
              row={row}
              top={v.start}
              selected={selection.ids}
              favorites={overrides}
              onOpen={onOpen}
              onToggle={onToggle}
              onFavorite={pickLocked ? undefined : onFavorite}
              onContextMenu={pickLocked ? undefined : onTileMenu}
              onDragStart={pickLocked ? undefined : onTileDrag}
              locked={pickLocked}
              compact={compact}
              hold={flying || scrubbing}
            />
          );
        })}
      </div>
      </div>
    </div>
      {empty && !isPlaceholder && summary?.itemCount === 0 && <div className="photo-grid__empty">{empty}</div>}
      <TimelineScrubber
        scroller={ref.current}
        sections={sections}
        total={virtualizer.getTotalSize()}
        grouping={scrubberGrouping}
        onScrollTo={(offset) => virtualizer.scrollToOffset(offset)}
        onDraggingChange={setScrubbing}
      />
    </div>
  );
}
