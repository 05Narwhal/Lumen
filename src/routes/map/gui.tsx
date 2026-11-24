import { Button, CloseButton, TextInput } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import {
  IconBuildingCommunity,
  IconBuildingCottage,
  IconBuildingSkyscraper,
  IconFlag,
  IconHome,
  IconMap,
  IconMap2,
  IconMapPin,
  IconMaximize,
  IconMountain,
  IconPlane,
  IconRipple,
  IconRoad,
  IconSearch,
  IconTrees,
  IconWifiOff,
  type Icon,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { MapView, type MapViewHandle } from "@/components/map-view";
import { PageHeader } from "@/components/page-header";
import { Segmented } from "@/components/segmented";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { useSettings, useSettingValue } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { searchPlaces, type PlaceKind, type PlaceResult } from "@/lib/place-search";
import { setViewerSequence } from "@/lib/viewer-sequence";
import { openAlbumPicker } from "@/components/album-picker/store";
import { useContextMenu } from "@/components/context-menu";
import { mapMenu } from "@/lib/menus";
import { runContribution, useContributions } from "@/lib/plugin-slots";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import type { MapStyle } from "@/types/generated/MapStyle";
import type { Bounds } from "@/types/generated/Bounds";
import type { Cluster } from "@/types/generated/Cluster";
import type { LibraryStats } from "@/types/generated/LibraryStats";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** One icon per kind of place, so a stream reads differently from a village at a glance. */
const PLACE_ICONS: Record<PlaceKind, Icon> = {
  country: IconFlag,
  region: IconMap2,
  city: IconBuildingSkyscraper,
  town: IconBuildingCommunity,
  village: IconBuildingCottage,
  water: IconRipple,
  mountain: IconMountain,
  nature: IconTrees,
  road: IconRoad,
  airport: IconPlane,
  landmark: IconMapPin,
  address: IconHome,
  place: IconMapPin,
};

/** Where the map opens before it knows anything: the whole world. */
const START = { lat: 20, lon: 0, zoom: 1 };

/** Room around a framed area, with the photo strip's height kept clear at the bottom. */
const FRAME = { top: 72, left: 72, right: 72, bottom: 72 + 112 };

/** How close *Open in Map* flies to a photo: a street, with its neighbours around it. */
const LOCATE_ZOOM = 15;

export function MapRoute() {
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const toolbarEntries = useContributions("map.toolbar");
  const map = useRef<MapViewHandle>(null);
  const [query, setQuery] = useState<{ bounds: Bounds; zoom: number } | null>(null);
  const [place, setPlace] = useState("");
  const [offline, setOffline] = useState(false);
  const { settings, update } = useSettings();
  const online = settings?.map.online ?? true;
  const [mapStyle, setMapStyle] = useSettingValue<MapStyle>(settings?.map.style ?? "map");

  const stats = useQuery({ queryKey: queryKeys.stats, queryFn: () => invoke<LibraryStats>("get_library_stats") });
  const extent = useQuery({
    queryKey: queryKeys.mapBounds(LIBRARY_SCOPE),
    queryFn: () => invoke<Bounds | null>("map_bounds", { scope: LIBRARY_SCOPE }),
  });

  const clusters = useQuery({
    queryKey: queryKeys.mapClusters(query?.bounds ?? null, query?.zoom ?? 0, LIBRARY_SCOPE),
    queryFn: () => invoke<Cluster[]>("map_clusters", { bounds: query?.bounds, zoom: query?.zoom, scope: LIBRARY_SCOPE }),
    enabled: query !== null,
    placeholderData: (prev) => prev,
  });

  const nearby = useQuery({
    queryKey: queryKeys.assetsInBounds(query?.bounds ?? null),
    queryFn: () => invoke<number[]>("assets_in_bounds", { bounds: query?.bounds, scope: LIBRARY_SCOPE, limit: 60 }),
    enabled: query !== null,
    placeholderData: (prev) => prev,
  });

  // Typing is debounced before it becomes a request: the online geocoder is a shared
  // service, and a request per keystroke is both rude and slower.
  const [placeQuery, setPlaceQuery] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setPlaceQuery(place.trim()), 250);
    return () => clearTimeout(t);
  }, [place]);
  const places = useQuery({
    queryKey: queryKeys.places(placeQuery, online),
    queryFn: () => searchPlaces(placeQuery, { online, limit: 8 }),
    enabled: placeQuery.length > 1,
    staleTime: 5 * 60_000,
  });
  const goToPlace = (p: PlaceResult) => {
    if (p.bounds) map.current?.fitBounds(p.bounds, FRAME);
    else map.current?.flyTo(p.lat, p.lon, Math.max(map.current.zoom(), 12));
    setPlace("");
  };

  // Frame the library the first time we learn where it is, unless a photo is being located.
  const locateId = Number(params.get("locate")) || null;
  const framed = useRef(false);
  useEffect(() => {
    if (framed.current || locateId != null || !extent.data || !map.current) return;
    map.current.fitBounds(extent.data, FRAME);
    framed.current = true;
  }, [extent.data, query, locateId]);

  // *Open in Map* (Round 26 B3) arrives as `?locate=<id>` (`?asset=` would open the viewer
  // over the map): fly to the photo, pin it, show its card, then drop the param in place so
  // Back returns to the viewer it came from.
  const located = useQuery({
    queryKey: queryKeys.asset(locateId ?? 0),
    queryFn: () => invoke<AssetDetail>("get_asset", { id: locateId }),
    enabled: locateId != null,
  });
  const [focus, setFocus] = useState<AssetDetail | null>(null);
  useEffect(() => {
    if (locateId == null || !map.current) return;
    const a = located.data;
    if (a?.id === locateId && a.gpsLat != null && a.gpsLon != null) {
      framed.current = true;
      map.current.flyTo(a.gpsLat, a.gpsLon, LOCATE_ZOOM);
      setFocus(a);
    } else if (!located.isError && a?.id !== locateId) return;
    setParams(
      (p) => {
        p.delete("locate");
        return p;
      },
      { replace: true },
    );
  }, [locateId, located.data, located.isError, extent.data, setParams]);

  const onBoundsChange = useCallback((bounds: Bounds, zoom: number) => setQuery({ bounds, zoom }), []);
  const show = useCallback(
    (id: number) =>
      setParams((p) => {
        p.set("asset", String(id));
        return p;
      }),
    [setParams],
  );
  // A photo opened from the map pages through the photos in view, not the whole library.
  const openAsset = useCallback(
    (id: number) => {
      const inView = nearby.data ?? [];
      setViewerSequence(inView.includes(id) ? inView : [id, ...inView]);
      show(id);
    },
    [nearby.data, show],
  );
  // A group opens as its own set, newest first, and the map zooms to it underneath, so
  // closing the viewer leaves you looking at where those photos were taken.
  const openGroup = useCallback(
    async (c: Cluster) => {
      map.current?.fitBounds(c.bounds, FRAME);
      const ids = await invoke<number[]>("assets_in_bounds", { bounds: c.bounds, scope: LIBRARY_SCOPE, limit: 500 }).catch(() => [c.coverAssetId]);
      const list = ids.length ? ids : [c.coverAssetId];
      setViewerSequence(list);
      show(list.includes(c.coverAssetId) ? c.coverAssetId : list[0]);
    },
    [show],
  );

  // A pin's own menu (Round 4G): its photos as a set, without opening them first.
  const { open: openMenu } = useContextMenu();
  const idsOf = useCallback(
    async (c: Cluster) =>
      c.assetId != null ? [c.assetId] : await invoke<number[]>("assets_in_bounds", { bounds: c.bounds, scope: LIBRARY_SCOPE, limit: 500 }).catch(() => [c.coverAssetId]),
    [],
  );
  const clusterMenu = (c: Cluster, e: React.MouseEvent) =>
    openMenu(
      e,
      mapMenu(c.assetId != null, {
        open: () => (c.assetId != null ? openAsset(c.assetId) : void openGroup(c)),
        slideshow: async () => {
          const ids = await idsOf(c);
          if (!ids.length) return;
          setViewerSequence(ids);
          setParams((p) => {
            p.set("asset", String(ids[0]));
            p.set("play", "1");
            return p;
          });
        },
        addToAlbum: async () => openAlbumPicker(await idsOf(c)),
        zoomToFit: () => map.current?.fitBounds(c.bounds, FRAME),
      }),
    );

  const total = useMemo(() => (clusters.data ?? []).reduce((n, c) => n + c.count, 0), [clusters.data]);
  const nothingMapped = extent.isSuccess && extent.data === null;

  return (
    <div className="map-route">
      <PageHeader
        title="Map"
        search={false}
        count={query && !nothingMapped ? `${total.toLocaleString()} in view` : undefined}
        toolbar={
          !nothingMapped && (
            <>
              <div className="map-route__search">
                <TextInput
                  size="xs"
                  leftSection={<IconSearch size={14} />}
                  placeholder="Find a place"
                  aria-label="Find a place"
                  value={place}
                  onChange={(e) => setPlace(e.currentTarget.value)}
                />
                {places.data && places.data.length > 0 && place.trim().length > 1 && (
                  <ul className="map-route__suggestions">
                    {places.data.map((p) => {
                      const KindIcon = PLACE_ICONS[p.kind];
                      return (
                        <li key={`${p.label}-${p.lat}-${p.lon}`}>
                          <button type="button" onClick={() => goToPlace(p)}>
                            <span className="map-route__place-icon" data-kind={p.kind} aria-hidden>
                              <KindIcon size={16} />
                            </span>
                            <span className="map-route__place-text">
                              <span className="map-route__place">{p.label}</span>
                              {p.detail && <span className="map-route__place-detail">{p.detail}</span>}
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
              {online && (
                <Segmented<MapStyle>
                  size="xs"
                  label="Map style"
                  value={mapStyle}
                  onChange={(v) => {
                    setMapStyle(v);
                    update((s) => ({ ...s, map: { ...s.map, style: v } }));
                  }}
                  data={[
                    { label: "Map", value: "map" },
                    { label: "Satellite", value: "satellite" },
                  ]}
                />
              )}
              <Button
                size="xs"
                variant="default"
                leftSection={<IconMaximize size={14} />}
                disabled={!extent.data}
                onClick={() => extent.data && map.current?.fitBounds(extent.data, FRAME)}
              >
                Fit library
              </Button>
              {toolbarEntries.map((c) => {
                const I = c.icon;
                return (
                  <Button key={c.id} size="xs" variant="default" leftSection={I ? <I size={14} /> : undefined} onClick={() => void runContribution(c, { navigate })}>
                    {c.label}
                  </Button>
                );
              })}
            </>
          )
        }
      />
      {nothingMapped ? (
        <EmptyState
          icon={<IconMap />}
          title="No photos with a location yet"
          // Say how many were checked. "Empty map" reads as a broken feature; "none of
          // your 45 photos carry coordinates" reads as a fact about the photos, which is
          // what it is. A camera with no GPS receiver still writes a GPS block holding
          // only a version number, so "it has GPS metadata" can be true and empty at once.
          description={
            stats.data
              ? `None of your ${stats.data.items.toLocaleString()} photos carry coordinates. A camera without a GPS receiver leaves them out — phone photos and GPS-tagged cameras appear here on their own.`
              : "Photos keep the place they were taken in their metadata. Import some from a phone or a camera with GPS and they appear here."
          }
        />
      ) : (
        <div className="map-route__body">
          <MapView
            ref={map}
            clusters={clusters.data ?? []}
            online={online}
            style={mapStyle}
            esriKey={settings?.map.esriKey}
            initial={START}
            onBoundsChange={onBoundsChange}
            onOpenAsset={openAsset}
            onZoomToCluster={(c) => void openGroup(c)}
            onClusterMenu={clusterMenu}
            onOfflineChange={setOffline}
            focus={focus && focus.gpsLat != null && focus.gpsLon != null ? { lat: focus.gpsLat, lon: focus.gpsLon } : null}
          />
          {focus && (
            <div className="map-route__card" role="group" aria-label={`${focus.filename} on the map`}>
              <button type="button" className="map-route__card-open" aria-label={`Open ${focus.filename}`} onClick={() => openAsset(focus.id)}>
                <img src={mediaUrl("thumb", focus.id, { s: 256 })} alt="" decoding="async" />
                <span className="map-route__card-text">
                  <span className="map-route__card-name">{focus.filename}</span>
                  <span className="map-route__card-sub">{focus.placeName ?? `${focus.gpsLat?.toFixed(4)}, ${focus.gpsLon?.toFixed(4)}`}</span>
                </span>
              </button>
              <CloseButton size="sm" aria-label="Close" onClick={() => setFocus(null)} />
            </div>
          )}
          {offline && (
            <div className="map-route__offline" role="status">
              <IconWifiOff size={14} />
              Street detail needs a connection. Showing the built-in world map.
            </div>
          )}
          {(nearby.data ?? []).length > 0 && (
            <div className="map-route__strip" aria-label="Photos in view">
              {(nearby.data ?? []).map((id) => (
                <button key={id} type="button" className="map-route__thumb" aria-label={`Open item ${id}`} onClick={() => openAsset(id)}>
                  <img src={mediaUrl("thumb", id, { s: 256 })} alt="" loading="lazy" decoding="async" />
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
