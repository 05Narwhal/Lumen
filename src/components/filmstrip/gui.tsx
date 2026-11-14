import { IconFlagFilled, IconX } from "@tabler/icons-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, type MouseEvent } from "react";
import { FLAG_PICK, FLAG_REJECT, LABELS } from "@/lib/marks";
import type { StripItem } from "@/types/generated/StripItem";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** Thumbnails are this tall; their width follows the photo, within limits. */
export const THUMB_H = 72;
const GAP = 4;

export const thumbWidth = (it: Pick<StripItem, "width" | "height">) => {
  if (!it.width || !it.height) return THUMB_H;
  return Math.round(Math.min(THUMB_H * 1.8, Math.max(THUMB_H * 0.6, (THUMB_H * it.width) / it.height)));
};

/** What a click with modifiers does to a selection, in strip order. */
export function nextSelection(ids: number[], selected: Set<number>, anchor: number | null, id: number, e: { ctrl: boolean; shift: boolean }): { ids: Set<number>; anchor: number | null } {
  if (e.shift && anchor !== null) {
    const a = ids.indexOf(anchor);
    const b = ids.indexOf(id);
    if (a >= 0 && b >= 0) {
      const [lo, hi] = a < b ? [a, b] : [b, a];
      return { ids: new Set([...selected, ...ids.slice(lo, hi + 1)]), anchor };
    }
  }
  if (e.ctrl) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return { ids: next, anchor: id };
  }
  return { ids: new Set(), anchor: id };
}

/**
 * Develop's filmstrip (spec §1): the photos in the top bar's order and filters, with their
 * flags, labels, ratings and whether they are edited. A click opens a photo, Ctrl/Cmd-click
 * adds it to the selection, Shift-click a run of them.
 */
export function Filmstrip({
  items,
  current,
  selected,
  hasMore,
  onLoadMore,
  onOpen,
  onSelect,
  onContextMenu,
}: {
  items: StripItem[];
  current: number;
  selected: Set<number>;
  hasMore: boolean;
  onLoadMore: () => void;
  onOpen: (id: number) => void;
  onSelect: (id: number, e: { ctrl: boolean; shift: boolean }) => void;
  onContextMenu?: (e: MouseEvent, item: StripItem) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    horizontal: true,
    count: items.length,
    getScrollElement: () => scroller.current,
    estimateSize: (i) => thumbWidth(items[i]) + GAP,
    overscan: 8,
    // Until the strip is measured (and where it cannot be), assume a wide window.
    initialRect: { width: 1600, height: THUMB_H + 32 },
  });
  const virtual = v.getVirtualItems();
  const lastShown = virtual.at(-1)?.index ?? 0;
  useEffect(() => {
    if (hasMore && lastShown >= items.length - 20) onLoadMore();
  }, [hasMore, lastShown, items.length, onLoadMore]);
  const currentIndex = items.findIndex((i) => i.id === current);
  useEffect(() => {
    if (currentIndex >= 0) v.scrollToIndex(currentIndex, { align: "auto" });
  }, [currentIndex, v]);

  return (
    <div ref={scroller} className="filmstrip" role="listbox" aria-label="Filmstrip" aria-multiselectable>
      <div className="filmstrip__track" style={{ width: v.getTotalSize() }}>
        {virtual.map((vi) => {
          const it = items[vi.index];
          const label = LABELS.find((l) => l.value === it.colorLabel);
          return (
            <button
              key={it.id}
              type="button"
              role="option"
              className="filmstrip__item"
              style={{ transform: `translateX(${vi.start}px)`, width: vi.size - GAP }}
              aria-label={it.filename}
              title={it.filename}
              aria-selected={selected.has(it.id) || it.id === current}
              aria-current={it.id === current ? "true" : undefined}
              data-reject={it.flag === FLAG_REJECT}
              onClick={(e) => {
                const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
                if (mods.ctrl || mods.shift) onSelect(it.id, mods);
                else {
                  onSelect(it.id, mods);
                  onOpen(it.id);
                }
              }}
              onContextMenu={onContextMenu ? (e) => onContextMenu(e, it) : undefined}
            >
              <img src={mediaUrl("thumb", it.id, { s: 256 })} alt="" loading="lazy" draggable={false} />
              {it.flag === FLAG_PICK && (
                <span className="filmstrip__flag" aria-label="Picked">
                  <IconFlagFilled size={11} />
                </span>
              )}
              {it.flag === FLAG_REJECT && (
                <span className="filmstrip__flag filmstrip__flag--reject" aria-label="Rejected">
                  <IconX size={11} />
                </span>
              )}
              {it.edited && <span className="filmstrip__edited" aria-label="Edited" />}
              {it.rating > 0 && <span className="filmstrip__stars" aria-label={`${it.rating} stars`}>{"★".repeat(it.rating)}</span>}
              {label && <span className="filmstrip__label" style={{ background: label.color }} aria-label={`${label.name} label`} />}
            </button>
          );
        })}
      </div>
    </div>
  );
}
