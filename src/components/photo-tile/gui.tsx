import { IconAppWindow, IconCheck, IconLivePhoto, IconPlayerPlay, IconStarFilled } from "@tabler/icons-react";
import { memo, useEffect, useRef, useState, type DragEvent, type MouseEvent } from "react";
import type { Tile } from "@/types/generated/Tile";
import { daysLeft } from "@/hooks/use-trash";
import { formatDuration } from "@/utils/date";
import { mediaUrl } from "@/utils/media";
import { thumbhashToDataUrl } from "@/utils/thumbhash";
import { useEditVersion } from "@/lib/edit-versions";
import { useExternalApps } from "@/lib/external-apps";
import "./styles.scss";

/** `assets.media_type` for the still half of a Live Photo (photos-core::assets). */
export const MEDIA_TYPE_LIVE = 2;

/**
 * Where the RAW tag sits: the icon of the app a copy was made for (Round 22 §8). Only a tile
 * that has one mounts it, so a grid of plain tiles asks nothing; an app that is gone gets a
 * plain window icon.
 */
function EditedInBadge({ appId }: { appId: string }) {
  const { apps } = useExternalApps();
  const app = apps.find((a) => a.id === appId);
  return (
    <span className="photo-tile__tag photo-tile__tag--app" title={`Edited in ${app?.name ?? "another app"}`}>
      {app ? <img className="photo-tile__app-icon" src={app.iconUrl} alt="" width={14} height={14} draggable={false} /> : <IconAppWindow size={14} />}
    </span>
  );
}

export interface TileModifiers {
  ctrl: boolean;
  shift: boolean;
}

export interface PhotoTileProps {
  tile: Tile;
  top: number;
  selected: boolean;
  /** Any selection is active: checkmarks stay visible and plain clicks toggle. */
  selecting: boolean;
  /** Overrides the tile's stored favorite flag while an optimistic update is pending. */
  favorite?: boolean;
  onOpen: (id: number) => void;
  onToggle: (id: number, mods: TileModifiers) => void;
  /** Double-click. The star burst is aimed at the tile itself, not at the pointer. */
  onFavorite?: (id: number) => void;
  onContextMenu?: (id: number, e: MouseEvent) => void;
  onDragStart?: (id: number, e: DragEvent<HTMLButtonElement>) => void;
  /** A thumbnail too small to carry badges (the year view): picture and check only. */
  compact?: boolean;
  /** Not yet: while true, a tile that has not started loading its image waits. Once started it keeps it. */
  hold?: boolean;
  /** Pick mode (Round 19 §5): a *locked* tile is already in the album and cannot be toggled. */
  pick?: "free" | "locked";
}

const DOUBLE_CLICK_MS = 220;

/**
 * `memo`'s test for a tile: every prop by identity except `tile`, compared field by field.
 * A grid refetch (a library change, every two seconds during an import) hands every tile a
 * new but identical object; comparing by identity re-rendered the whole grid each time.
 */
export function tilePropsEqual(a: PhotoTileProps, b: PhotoTileProps): boolean {
  for (const k of Object.keys({ ...a, ...b }) as (keyof PhotoTileProps)[]) {
    if (k === "tile") continue;
    if (!Object.is(a[k], b[k])) return false;
  }
  if (a.tile === b.tile) return true;
  const ta = a.tile as unknown as Record<string, unknown>;
  const tb = b.tile as unknown as Record<string, unknown>;
  const keys = Object.keys(ta);
  return keys.length === Object.keys(tb).length && keys.every((k) => Object.is(ta[k], tb[k]));
}

/**
 * Which derivative to request for a tile of this size. Chosen from the longest side in
 * physical pixels, not from the height: a landscape M tile is ~300 px wide at 200 px tall,
 * so picking on height alone stretched a 256 px thumbnail across it and looked soft.
 */
export function thumbSize(w: number, h: number): 256 | 1024 {
  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  return Math.max(w, h) * dpr > 256 ? 1024 : 256;
}

/**
 * How long a tile waits before asking again after a failed load. A 1024 px thumbnail is made on
 * demand and the backend stops waiting for it after 30 s while the render goes on, so the file
 * is usually there by the next ask.
 */
const RETRY_MS = [2000, 8000, 30000];

/**
 * One grid tile. Deliberately plain: no Framer Motion here, ever. Paint order is
 * dominant colour → ThumbHash placeholder → <img> fading in on decode.
 */
export const PhotoTile = memo(function PhotoTile({
  tile,
  top,
  selected,
  selecting,
  favorite,
  onOpen,
  onToggle,
  onFavorite,
  onContextMenu,
  onDragStart,
  compact = false,
  hold = false,
  pick,
}: PhotoTileProps) {
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (retry.current) clearTimeout(retry.current);
  }, []);
  const onImageError = () => {
    const wait = RETRY_MS[attempt];
    if (wait === undefined) setFailed(true);
    else retry.current = setTimeout(() => setAttempt((n) => n + 1), wait);
  };
  const [started, setStarted] = useState(!hold);
  useEffect(() => {
    if (!hold) setStarted(true);
  }, [hold]);
  // Subscribes the tile to its edit version, which `mediaUrl` adds to the image URL.
  useEditVersion(tile.id);
  const size = thumbSize(tile.w, tile.h);
  // A 56 px Years tile shows its colour while loading: decoding a ThumbHash for each of the
  // thousand tiles that scroll past cost more than the frame had. So does a tile the
  // scrubber drags past (Round 4H): it waits, coloured, until the drag ends.
  const placeholder = tile.thumbhash && !compact && started ? thumbhashToDataUrl(tile.thumbhash) : "";
  const isFavorite = favorite ?? tile.isFavorite;
  const pendingOpen = useRef<ReturnType<typeof setTimeout> | null>(null);

  // A single click opens after a short wait so a double-click (favorite) can cancel it.
  const onClick = (e: MouseEvent) => {
    if (pick === "locked") return;
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    if (mods.ctrl || mods.shift || selecting) {
      onToggle(tile.id, mods);
      return;
    }
    if (pendingOpen.current) clearTimeout(pendingOpen.current);
    pendingOpen.current = setTimeout(() => {
      pendingOpen.current = null;
      onOpen(tile.id);
    }, DOUBLE_CLICK_MS);
  };
  const onDoubleClick = () => {
    if (pendingOpen.current) {
      clearTimeout(pendingOpen.current);
      pendingOpen.current = null;
    }
    onFavorite?.(tile.id);
  };

  return (
    <div
      className="photo-tile"
      data-part="tile"
      id={`tile-${tile.id}`}
      style={{
        left: tile.x,
        top,
        width: tile.w,
        height: tile.h,
        backgroundColor: tile.color ?? undefined,
        backgroundImage: placeholder ? `url(${placeholder})` : undefined,
      }}
      data-selected={selected}
      data-locked={pick === "locked" || undefined}
      data-compact={compact || undefined}
      data-favorite={isFavorite}
      data-loaded={loaded}
      data-loading={tile.renderable && !loaded && !failed && !placeholder}
      data-renderable={tile.renderable}
    >
      <button
        type="button"
        className="photo-tile__hit"
        aria-label={`Item ${tile.id}`}
        onClick={onClick}
        onDoubleClick={onDoubleClick}
        onContextMenu={(e) => onContextMenu?.(tile.id, e)}
        draggable={!!onDragStart}
        onDragStart={(e) => onDragStart?.(tile.id, e)}
      >
        {tile.renderable && !failed && started && (
          <img
            key={attempt}
            className="photo-tile__img"
            src={mediaUrl("thumb", tile.id, { s: size })}
            alt=""
            decoding="async"
            loading="lazy"
            draggable={false}
            onLoad={() => setLoaded(true)}
            onError={onImageError}
          />
        )}
        {/* A Years square has no room for badges: they are not rendered at all, which is
            also most of what mounting a row of them cost while scrolling. */}
        {!compact && (
          <>
            {(!tile.renderable || failed) && (
              <span className="photo-tile__format" aria-hidden>
                {tile.extension.toUpperCase()}
              </span>
            )}
            {pick === "locked" && <span className="photo-tile__locked">In album</span>}
            {tile.trashedAt != null && (
              <span
                className="photo-tile__countdown"
                title={`${daysLeft(tile.trashedAt)} days left`}
                aria-label={`${daysLeft(tile.trashedAt)} days left`}
              >
                {daysLeft(tile.trashedAt)}d
              </span>
            )}
            {/* What kind of original this is. Narrow tiles step down by their own width (styles.scss
                container queries) instead of dropping the badges: words, then compact, then a dot. */}
            {(tile.raw || tile.hdr > 0 || tile.editedIn) && (
              <span className="photo-tile__tags">
                {tile.raw && (
                  <span className="photo-tile__tag" title="Camera RAW original">
                    RAW
                  </span>
                )}
                {tile.hdr > 0 && (
                  <span className="photo-tile__tag" title="High dynamic range">
                    HDR
                  </span>
                )}
                {tile.editedIn && <EditedInBadge appId={tile.editedIn} />}
              </span>
            )}
            {tile.rating > 0 && tile.h >= 160 && (
              <span className="photo-tile__rating" aria-label={`Rated ${tile.rating} of 5`}>
                {"\u2605".repeat(tile.rating)}
              </span>
            )}
            {isFavorite && (
              <span className="photo-tile__star" aria-label="Favorite">
                <IconStarFilled size={14} />
              </span>
            )}
            {tile.mediaType === 1 && (
              <span className="photo-tile__badge">
                <IconPlayerPlay size={11} />
                {tile.durationMs != null && <span className="photo-tile__badge-text">{formatDuration(tile.durationMs)}</span>}
              </span>
            )}
            {tile.mediaType === MEDIA_TYPE_LIVE && (
              <span className="photo-tile__badge photo-tile__badge--live" title="Live Photo">
                <IconLivePhoto size={12} />
                <span className="photo-tile__badge-text">LIVE</span>
              </span>
            )}
          </>
        )}
      </button>
      {/* Years squares show the checkbox only while selecting; Ctrl and Shift click still select. */}
      {(!compact || selecting) && (
        <button
          type="button"
          className="photo-tile__check"
          aria-label={selected ? "Deselect" : "Select"}
          aria-pressed={selected}
          disabled={pick === "locked"}
          data-visible={selecting}
          onClick={(e) => {
            e.stopPropagation();
            onToggle(tile.id, { ctrl: true, shift: e.shiftKey });
          }}
        >
          <IconCheck size={14} stroke={3} />
        </button>
      )}
    </div>
  );
}, tilePropsEqual);
