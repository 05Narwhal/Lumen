import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { IconPhoto } from "@tabler/icons-react";
import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { PhotoGrid } from "@/components/photo-grid";
import { Segmented } from "@/components/segmented";
import { TagBar } from "@/components/tag-search";
import { useAlbums } from "@/hooks/use-albums";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { clearSelection } from "@/hooks/use-selection";
import { pickAlbumId } from "@/lib/pick-mode";
import { GROUP_LABELS } from "@/components/sidebar";
import type { DeviceGroup } from "@/types/generated/DeviceGroup";
import { useDevices } from "@/hooks/use-devices";
import { isEmptyQuery, useSearch } from "@/hooks/use-search";
import { useSettings, useSettingValue } from "@/hooks/use-settings";
import { collectionFor, collectionForTags, queryToTags } from "@/lib/collections";
import { appendSeq, EMPTY_SEQ, flatTags, fromLegacy, plainTags } from "@/lib/search-expr";
import { parseTags, writeTags } from "@/lib/search-tags";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { GroupBy } from "@/types/generated/GroupBy";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { LibraryStats } from "@/types/generated/LibraryStats";
import { TextEmptyState } from "@/components/text-empty";
import { hasTextTag } from "@/lib/photo-text";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { TileSize } from "@/types/generated/TileSize";
import { LIBRARY_SOURCE, PickBar, PickTree, pickScope, usePickAlbum, usePickLocked, type PickSource } from "./pick";
import "./styles.scss";

/** Target row heights per tile size (UX spec §6.2). */
export const ROW_HEIGHTS: Record<TileSize, number> = { s: 120, m: 200, l: 320 };

/** The side of a square in the year view: small enough to take in a year at a glance. */
export const YEAR_TILE = 56;

/** The sizes each grouping offers. A year has one, so it offers no choice at all. */
const SIZES_FOR: Record<GroupBy, TileSize[]> = { day: ["s", "m", "l"], month: ["s", "m"], year: [] };

const SIZE_LABELS: Record<TileSize, string> = { s: "S", m: "M", l: "L" };

/** A saved size the grouping does not offer is shown as the largest one it does. */
function sizeWithin(group: GroupBy, size: TileSize): TileSize {
  const offered = SIZES_FOR[group];
  return offered.length === 0 || offered.includes(size) ? size : offered[offered.length - 1];
}

export function LibraryRoute() {
  const { settings, update } = useSettings();
  const [searchParams, setSearchParams] = useSearchParams();
  const { deviceId, group } = useParams();
  const { devices } = useDevices();
  // A device group (`/library/group/iphone`) is its devices as a filter. Until the devices
  // load, an impossible id keeps the grid from showing the whole library for a moment.
  const groupIds = useMemo(() => {
    if (!group) return null;
    const ids = devices.filter((d) => d.group === group).map((d) => d.id);
    return ids.length ? ids : [0];
  }, [group, devices]);
  // The page's search tags ride in its query (`tagExpr`), with whatever a link set.
  const { query, active } = useSearch();
  const seq = query.tagExpr ?? EMPTY_SEQ;
  // A link from before Round 7 (`?q=beach`, `?filters={"favorites":true}`) becomes tags, so
  // the bar shows it and takes it away like any other.
  const legacy = searchParams.get("filters") ?? searchParams.get("q");
  useEffect(() => {
    if (!legacy) return;
    setSearchParams(
      (p) => {
        let filters: Partial<AssetQuery> = {};
        try {
          filters = JSON.parse(p.get("filters") ?? "{}") as Partial<AssetQuery>;
        } catch {
          // A hand-edited URL keeps what it can.
        }
        const converted = queryToTags({ ...filters, text: p.get("q") });
        if (!converted) return p;
        // Old links narrowed with the old rule; their tags are appended as one more `and` run.
        writeTags(p, appendSeq(parseTags(p), fromLegacy(converted), "and"));
        p.delete("filters");
        p.delete("q");
        return p;
      },
      { replace: true },
    );
  }, [legacy, setSearchParams]);
  // Round 19 §5: `?pick=album:<id>` is *Add photos* — the same grid, choosing instead of opening.
  const navigate = useNavigate();
  const pickId = pickAlbumId(searchParams.toString());
  const [source, setSource] = useState<PickSource>(LIBRARY_SOURCE);
  const { addTo } = useAlbums();
  const pickAlbum = usePickAlbum(pickId ?? 0);
  const locked = usePickLocked(pickId ?? 0);
  useEffect(() => {
    if (pickId === null) return;
    clearSelection();
    return () => clearSelection();
  }, [pickId]);
  const scope = useMemo<LayoutScope>(() => {
    const q = groupIds ? { ...query, deviceIds: groupIds } : isEmptyQuery(query) ? null : query;
    if (pickId !== null) return pickScope(source, q);
    return { ...LIBRARY_SCOPE, deviceId: deviceId ? Number(deviceId) : null, query: q };
  }, [deviceId, query, groupIds, pickId, source]);
  const backToAlbum = useCallback(() => navigate(`/albums/${pickId}`), [navigate, pickId]);
  const addPicked = useCallback(
    async (ids: number[]) => {
      if (pickId === null) return;
      await addTo({ albumId: pickId, assetIds: ids, albumName: pickAlbum.data?.name });
      clearSelection();
      navigate(`/albums/${pickId}`);
    },
    [addTo, navigate, pickAlbum.data?.name, pickId],
  );
  const matches = useQuery({
    queryKey: queryKeys.count(scope),
    queryFn: () => invoke<number>("count_scope", { scope }),
    enabled: active,
  });
  const stats = useQuery({ queryKey: queryKeys.stats, queryFn: () => invoke<LibraryStats>("get_library_stats") });
  // The selector shows its own value at once and the grid follows the deferred one, so
  // the pill glides while the tiles catch up.
  const [storedTileSize, setTileSize] = useSettingValue(settings?.appearance.tileSize ?? "m");
  const [groupBy, setGroupBy] = useSettingValue<GroupBy>(settings?.appearance.libraryGroupBy ?? "month");
  const tileSize = sizeWithin(groupBy, storedTileSize);
  const gridTileSize = useDeferredValue(tileSize);
  const gridGroupBy = useDeferredValue(groupBy);
  const rowHeight = gridGroupBy === "year" ? YEAR_TILE : ROW_HEIGHTS[gridTileSize];
  // A photo to land on after switching grouping — set by a click in the year view.
  const [focusAsset, setFocusAsset] = useState<number | null>(null);
  const clearFocus = useCallback(() => setFocusAsset(null), []);
  const chooseGroup = useCallback(
    (v: GroupBy) => {
      setGroupBy(v);
      update((s) => ({ ...s, appearance: { ...s.appearance, libraryGroupBy: v } }));
    },
    [setGroupBy, update],
  );
  // Memoised on the value alone. When the settings round trip lands a couple of frames
  // after the click, this route re-renders; without an identical element React would
  // re-render the control, Mantine would rewrite the indicator's inline transform, and
  // that cancels the transition already in flight. Measured: the indicator starts two
  // transitions on click and has none two frames later.
  const sizeControl = useMemo(
    () => (
      <div className="library-route__toolbar">
        <Segmented<GroupBy>
          size="xs"
          label="Group by"
          value={groupBy}
          onChange={chooseGroup}
          data={[
            { label: "Days", value: "day" },
            { label: "Months", value: "month" },
            { label: "Years", value: "year" },
          ]}
        />
        {SIZES_FOR[groupBy].length > 0 && (
          <Segmented<TileSize>
            size="xs"
            label="Tile size"
            value={tileSize}
            onChange={(v) => {
              setTileSize(v);
              update((s) => ({ ...s, appearance: { ...s.appearance, tileSize: v } }));
            }}
            data={SIZES_FOR[groupBy].map((v) => ({ label: SIZE_LABELS[v], value: v }))}
          />
        )}
      </div>
    ),
    [groupBy, chooseGroup, tileSize, setTileSize, update],
  );
  const device = deviceId ? devices.find((d) => d.id === Number(deviceId)) : undefined;

  const onOpen = useCallback(
    (id: number) => {
      // The year view is for finding your way: a photo there opens its month, not the viewer.
      if (gridGroupBy === "year") {
        chooseGroup("month");
        setFocusAsset(id);
        return;
      }
      setSearchParams((p) => {
        p.set("asset", String(id));
        return p;
      });
    },
    [gridGroupBy, chooseGroup, setSearchParams],
  );

  const empty = stats.data !== undefined && stats.data.items === 0;

  return (
    <div className="library-route">
      <PageHeader
        count={active && matches.data != null ? `${matches.data.toLocaleString()} match` : undefined}
        title={
          pickId !== null ? (
            "Add photos"
          ) : device ? (
            <span className="library-route__device-title">
              {device.displayName}
              <span className="library-route__device-count">{device.itemCount.toLocaleString()} items</span>
            </span>
          ) : group && group in GROUP_LABELS ? (
            <span className="library-route__device-title">
              {GROUP_LABELS[group as DeviceGroup]}
              <span className="library-route__device-count">
                {`${devices
                  .filter((d) => d.group === group)
                  .reduce((n, d) => n + d.itemCount, 0)
                  .toLocaleString()} items`}
              </span>
            </span>
          ) : (
            // Opened from a collection card, the page is that collection.
            (collectionForTags(plainTags(seq) ?? [])?.label ?? collectionFor({ ...query, searchTags: [], tagExpr: undefined })?.label ?? "Library")
          )
        }
        toolbar={!empty && sizeControl}
        // Every Library page filters in place by tags (Round 7).
        search="tags"
      />
      {empty ? (
        <EmptyState
          icon={<IconPhoto />}
          title="No photos yet"
          description="Import a folder, a memory card or an iPhone to start your library."
          action={
            <Button component={Link} to="/import">
              Import photos
            </Button>
          }
        />
      ) : (
        <div className="library-route__body" data-pick={pickId !== null || undefined}>
          {pickId !== null && <PickTree albumId={pickId} source={source} onSource={setSource} />}
          <div className="library-route__main">
          {pickId !== null && <PickBar album={pickAlbum.data} locked={locked} onCancel={backToAlbum} onAdd={(ids) => void addPicked(ids)} />}
          <TagBar />
          <div className="library-route__grid">
            <PhotoGrid
              pick={pickId !== null ? { locked } : undefined}
              rowHeight={rowHeight}
              scope={scope}
              groupBy={gridGroupBy}
              // Only once the grid has caught up with the grouping it is meant to land in.
              focusAsset={gridGroupBy === groupBy ? focusAsset : null}
              onFocused={clearFocus}
              onOpen={onOpen}
              empty={hasTextTag(flatTags(seq)) ? <TextEmptyState /> : undefined}
            />
          </div>
          </div>
        </div>
      )}
    </div>
  );
}
