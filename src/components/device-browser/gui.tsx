import { WindowChrome } from "@/components/window-chrome";
import { Button, Modal } from "@mantine/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import { IconCheck, IconDeviceMobileOff, IconFilterOff, IconPlayerPlayFilled } from "@tabler/icons-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { EmptyState } from "@/components/empty-state";
import { intersects, normalizeRect, type Rect } from "@/components/photo-grid/marquee";
import { toast } from "@/components/toast";
import { useDeviceBrowser } from "@/hooks/use-device-browser";
import { isAppError } from "@/lib/ipc";
import type { BrowseTarget } from "@/types/generated/BrowseTarget";
import type { BrowserItem } from "@/types/generated/BrowserItem";
import { formatBytes, formatStorage } from "@/utils/format";
import { mediaUrl } from "@/utils/media";
import { Filters } from "./filters";
import { buildRows, columnsFor, DEFAULT_FILTER, type Row } from "./model";
import "./styles.scss";

/** Square tiles, in CSS pixels; `GAP` matches the narrowest `--tile-gap` (the grid lays out in JS). */
export const TILE = 120;
const GAP = 4;
const HEADER = 36;
const PAD = 16;
/** Load the next page this many rows before the end. */
const AHEAD = 6;
/** A press that moves less than this is a click, not a drag. */
const DRAG_SLOP = 4;

const n = (v: number) => v.toLocaleString();

const KIND_BADGE: Record<string, string | null> = { photo: null, video: null, raw: "RAW", live: "LIVE" };

const Tile = memo(function Tile({
  item,
  selected,
  onClick,
  onFocus,
}: {
  item: BrowserItem;
  selected: boolean;
  onClick: (item: BrowserItem, e: ReactMouseEvent) => void;
  onFocus: (item: BrowserItem) => void;
}) {
  const [state, setState] = useState<"loading" | "loaded" | "failed">("loading");
  const badge = KIND_BADGE[item.kind];
  return (
    <button
      type="button"
      className="device-browser__tile"
      data-selected={selected}
      data-imported={item.imported}
      aria-pressed={selected}
      aria-label={`${item.fileName}${item.imported ? ", already imported" : ""}`}
      onClick={(e) => onClick(item, e)}
      onFocus={() => onFocus(item)}
      data-index={item.index}
    >
      {state !== "failed" && (
        <img
          src={mediaUrl("device-thumb", item.token)}
          alt={item.fileName}
          loading="lazy"
          draggable={false}
          data-loaded={state === "loaded"}
          onLoad={() => setState("loaded")}
          onError={() => setState("failed")}
        />
      )}
      {item.kind === "video" && (
        <span className="device-browser__badge" aria-hidden>
          <IconPlayerPlayFilled size={11} />
        </span>
      )}
      {badge && (
        <span className="device-browser__badge" aria-hidden>
          {badge}
        </span>
      )}
      {item.imported && (
        <span className="device-browser__imported" aria-hidden title="Already in your library">
          <IconCheck size={11} />
        </span>
      )}
      <span className="device-browser__check" aria-hidden>
        <IconCheck size={14} />
      </span>
    </button>
  );
});

/**
 * Everything on a device, to pick exactly what to import (Round 4E): a full-window sheet over
 * the connected page with filters along the top, a virtualised grid by day, and the selection
 * with *Import* along the bottom. Esc closes it; Space shows the focused item larger.
 */
export function DeviceBrowser({
  target,
  title,
  onClose,
  onImportStarted,
}: {
  target: BrowseTarget;
  title: string;
  onClose: () => void;
  onImportStarted?: (batchId: number) => void;
}) {
  const b = useDeviceBrowser(target);
  // Measured once the modal has mounted its content (a ref read on the first effect would
  // still be empty), and again on every resize.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [gridEl, setGridEl] = useState<HTMLDivElement | null>(null);
  const attachGrid = useCallback((el: HTMLDivElement | null) => {
    scrollRef.current = el;
    setGridEl(el);
  }, []);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    if (!gridEl) return;
    const apply = () => setSize({ width: gridEl.clientWidth, height: gridEl.clientHeight });
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(gridEl);
    return () => ro.disconnect();
  }, [gridEl]);
  const columns = columnsFor(Math.max(0, size.width - PAD * 2), TILE, GAP);
  const rows = useMemo(() => buildRows(b.items, columns), [b.items, columns]);
  const [focused, setFocused] = useState<BrowserItem | null>(null);
  const [preview, setPreview] = useState<BrowserItem | null>(null);
  const [starting, setStarting] = useState(false);
  const [band, setBand] = useState<Rect | null>(null);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (rows[i]?.kind === "header" ? HEADER : TILE + GAP),
    overscan: 6,
    initialRect: { width: 1280, height: 800 },
  });
  useEffect(() => virtualizer.measure(), [rows, virtualizer]);
  const virtualRows = virtualizer.getVirtualItems();
  const lastShown = virtualRows.length ? virtualRows[virtualRows.length - 1].index : -1;
  const { hasMore, loadingMore, loadMore } = b;
  useEffect(() => {
    if (hasMore && !loadingMore && lastShown >= rows.length - AHEAD) void loadMore();
  }, [hasMore, loadingMore, loadMore, lastShown, rows.length]);

  const click = b.click;
  // Set while a drag draws a band, so the click that ends it does not toggle a tile.
  const dragged = useRef(false);
  const onTileClick = useCallback(
    (item: BrowserItem, e: ReactMouseEvent) => {
      if (dragged.current) return;
      click(item.index, { shift: e.shiftKey, toggle: e.ctrlKey || e.metaKey });
      setFocused(item);
    },
    [click],
  );
  const onTileFocus = useCallback((item: BrowserItem) => setFocused(item), []);

  // A drag from anywhere in the grid draws a band; tiles it touches join the selection.
  const drag = useRef<{ x: number; y: number; scroll: number } | null>(null);
  const toContent = (el: HTMLElement, clientX: number, clientY: number) => {
    const r = el.getBoundingClientRect();
    return { x: clientX - r.left - PAD, y: clientY - r.top + el.scrollTop };
  };
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !scrollRef.current) return;
    dragged.current = false;
    const p = toContent(scrollRef.current, e.clientX, e.clientY);
    drag.current = { x: p.x, y: p.y, scroll: scrollRef.current.scrollTop };
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = scrollRef.current;
    const start = drag.current;
    if (!el || !start || (e.buttons & 1) === 0) return;
    const p = toContent(el, e.clientX, e.clientY);
    if (!dragged.current && Math.hypot(p.x - start.x, p.y - start.y) < DRAG_SLOP) return;
    dragged.current = true;
    setBand(normalizeRect(start.x, start.y, p.x, p.y));
  };
  const onPointerUp = () => {
    if (band) {
      const hit: number[] = [];
      for (const v of virtualizer.getVirtualItems()) {
        const row = rows[v.index];
        if (row?.kind !== "tiles") continue;
        row.items.forEach((item, col) => {
          if (intersects(band, { x: col * (TILE + GAP), y: v.start, w: TILE, h: TILE })) hit.push(item.index);
        });
      }
      if (hit.length) b.selectMany(hit, true);
    }
    setBand(null);
    drag.current = null;
    // The click that ends a drag must not toggle the tile under the pointer.
    setTimeout(() => (dragged.current = false), 0);
  };

  const importNow = async () => {
    setStarting(true);
    try {
      const batch = await b.startImport();
      if (batch !== null) {
        onImportStarted?.(batch);
        onClose();
      }
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setStarting(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === " " && focused && !(e.target instanceof HTMLInputElement)) {
      e.preventDefault();
      setPreview((p) => (p ? null : focused));
    } else if (e.key === "Escape" && preview) {
      e.stopPropagation();
      setPreview(null);
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a" && !(e.target instanceof HTMLInputElement)) {
      e.preventDefault();
      b.selectAll();
    }
  };

  const s = b.session;
  const sel = b.selection;
  const toggleDay = (row: Extract<Row, { kind: "header" }>) => b.selectMany(row.ids, !row.ids.every((i) => b.selected.has(i)));

  let body: React.ReactNode = null;
  if (b.gone) {
    body = (
      <EmptyState
        icon={<IconDeviceMobileOff size={28} />}
        title="The device was disconnected"
        description="Connect it again to keep browsing; nothing on it was changed."
        action={<Button onClick={onClose}>Close</Button>}
      />
    );
  } else if (b.error) {
    body = (
      <EmptyState
        icon={<IconDeviceMobileOff size={28} />}
        title="It could not be read"
        description={isAppError(b.error) ? b.error.message : "The device's photos could not be read."}
        action={<Button onClick={b.retry}>Try again</Button>}
      />
    );
  } else if (b.loading) {
    body = <p className="device-browser__status">Reading what is on it…</p>;
  } else if (b.items.length === 0 && b.matching === 0 && !b.loadingMore) {
    body = (
      <EmptyState
        icon={<IconFilterOff size={28} />}
        title={s?.total ? "Nothing matches these filters" : "No photos or videos on it"}
        description={s?.total ? `${n(s.total)} items are on it under other dates or kinds.` : "There is nothing here to import."}
        action={s?.total ? <Button onClick={() => b.setFilter(DEFAULT_FILTER)}>Show everything</Button> : undefined}
      />
    );
  }

  return (
    <Modal
      opened
      onClose={onClose}
      title={s?.name ?? title}
      fullScreen
      classNames={{ content: "device-browser", body: "device-browser__body", title: "device-browser__title" , header: "device-browser__header" }}
      closeOnEscape={!preview}
    >
      <WindowChrome overModal />
      <div className="device-browser__frame" onKeyDown={onKeyDown}>
        <div className="device-browser__top">
          <Filters session={s} filter={b.filter} onChange={b.setFilter} />
          <span className="device-browser__totals">
            {s ? `${n(b.matching)} of ${n(s.total)} · ${formatStorage(s.totalBytes)} on the device` : ""}
            {s?.dating && <span className="device-browser__dating"> · Reading dates…</span>}
          </span>
        </div>

        <div
          ref={attachGrid}
          className="device-browser__grid"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerLeave={() => band && onPointerUp()}
          aria-label="Items on the device"
        >
          {body ?? (
            <div className="device-browser__canvas" style={{ height: virtualizer.getTotalSize() }}>
              {virtualRows.map((v) => {
                const row = rows[v.index];
                if (!row) return null;
                return (
                  <div key={row.key} className="device-browser__row" style={{ transform: `translateY(${v.start}px)`, height: v.size }}>
                    {row.kind === "header" ? (
                      <div className="device-browser__day">
                        <h3 className="device-browser__day-label">{row.label}</h3>
                        <Button size="compact-xs" variant="subtle" onClick={() => toggleDay(row)}>
                          {row.ids.every((i) => b.selected.has(i)) ? "Deselect day" : "Select day"}
                        </Button>
                      </div>
                    ) : (
                      row.items.map((item) => (
                        <Tile key={item.index} item={item} selected={b.selected.has(item.index)} onClick={onTileClick} onFocus={onTileFocus} />
                      ))
                    )}
                  </div>
                );
              })}
              {band && (
                <div className="device-browser__band" style={{ left: band.x, top: band.y, width: band.w, height: band.h }} aria-hidden />
              )}
            </div>
          )}
        </div>

        <footer className="device-browser__bar">
          <Button variant="default" size="xs" onClick={b.selectAll} disabled={!s || b.matching === 0}>
            Select all ({n(b.matching)})
          </Button>
          <Button variant="subtle" size="xs" onClick={b.selectNone} disabled={sel.count === 0}>
            Select none
          </Button>
          <Button variant="subtle" size="xs" onClick={b.invert} disabled={!s || b.matching === 0}>
            Invert
          </Button>
          <span className="device-browser__count" aria-live="polite">
            {sel.count === 0
              ? "Nothing selected"
              : [
                  `${n(sel.count)} selected`,
                  formatBytes(sel.bytes),
                  sel.imported > 0 ? `${n(sel.imported)} already in your library` : null,
                  b.hiddenSelected > 0 ? `… and ${n(b.hiddenSelected)} not shown` : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
          </span>
          <Button onClick={() => void importNow()} loading={starting} disabled={sel.count === 0 || b.gone}>
            {sel.count === 0 ? "Import" : `Import ${n(sel.count)}`}
          </Button>
        </footer>

        {preview && (
          <div className="device-browser__preview" role="dialog" aria-label={preview.fileName} onClick={() => setPreview(null)}>
            <img src={mediaUrl("device-thumb", preview.token, { s: 1024 })} alt={preview.fileName} />
            <span className="device-browser__preview-name">{preview.fileName}</span>
          </div>
        )}
      </div>
    </Modal>
  );
}
