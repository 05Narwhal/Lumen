// Round 19 §5: the Library in pick mode (`/library?pick=album:<id>`) — a source tree on the
// left, a pinned bar above the grid. The grid, search, tags and filters are the Library's own.
import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { IconAlbum, IconChevronRight, IconFolder, IconMapPin, IconPaw, IconPhoto, IconStar, IconUser, IconUsers } from "@tabler/icons-react";
import { motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { PathName } from "@/components/album-name";
import { Fold } from "@/components/fold";
import { IconCover } from "@/components/icon-cover";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { usePeople } from "@/hooks/use-people";
import { usePets, petName } from "@/hooks/use-pets";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { selectionStore, useSelection } from "@/hooks/use-selection";
import { albumPath, contentsOf } from "@/lib/album-collections";
import { invoke } from "@/lib/ipc";
import { mediaUrl } from "@/utils/media";
import { useOpenSet } from "@/lib/open-set";
import { queryKeys } from "@/lib/query-keys";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { CoverStyle } from "@/types/generated/CoverStyle";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { PlaceRef } from "@/types/generated/PlaceRef";
import type { SearchOverview } from "@/types/generated/SearchOverview";

/** Where the grid takes its photos from while picking. */
export type PickSource =
  | { kind: "library" }
  | { kind: "favorites" }
  | { kind: "album"; id: number }
  | { kind: "person"; id: number }
  | { kind: "pet"; id: number }
  | { kind: "place"; place: PlaceRef; label: string };

export const LIBRARY_SOURCE: PickSource = { kind: "library" };

const same = (a: PickSource, b: PickSource) => JSON.stringify(a) === JSON.stringify(b);

/** The grid's scope for a source, with the page's search `query` (its tags) laid over it. */
export function pickScope(source: PickSource, query: AssetQuery | null): LayoutScope {
  const base: LayoutScope = { ...LIBRARY_SCOPE, query };
  switch (source.kind) {
    case "favorites":
      return { ...base, favorites: true };
    case "album":
      return { ...base, albumId: source.id };
    case "person":
      return { ...base, personId: source.id };
    case "pet":
      return { ...base, petId: source.id };
    case "place":
      return { ...base, query: { ...(query ?? ({} as AssetQuery)), places: [source.place] } as AssetQuery };
    default:
      return base;
  }
}

const NOBODY: ReadonlySet<number> = new Set();

/** The photos already in the album: shown checked and locked. */
export function usePickLocked(albumId: number): ReadonlySet<number> {
  const ids = useQuery({
    queryKey: ["pick-locked", albumId],
    queryFn: () => invoke<number[]>("scope_ids", { scope: { ...LIBRARY_SCOPE, albumId }, limit: 200_000 }),
    staleTime: 0,
  });
  return useMemo(() => (ids.data ? new Set(ids.data) : NOBODY), [ids.data]);
}

/** The picked photos that are not already in the album. */
export function pickedIds(locked: ReadonlySet<number>): number[] {
  return [...selectionStore.get().ids].filter((id) => !locked.has(id));
}

function isEditable(t: EventTarget | null) {
  return t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
}

/** *Adding to …*, *Cancel* and *Add N photos*; `Esc` cancels and `Enter` adds. */
export function PickBar({
  album,
  locked,
  onCancel,
  onAdd,
}: {
  album: AlbumSummary | undefined;
  locked: ReadonlySet<number>;
  onCancel: () => void;
  onAdd: (ids: number[]) => void;
}) {
  const { collections } = useAlbumCollections();
  const { ids } = useSelection();
  const picked = useMemo(() => [...ids].filter((id) => !locked.has(id)), [ids, locked]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || isEditable(e.target)) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
        return;
      }
      if (e.key === "Enter") {
        // A focused button outside the grid (the tree, Cancel) keeps its own Enter.
        const t = e.target as HTMLElement;
        if (t.closest("button") && !t.closest(".photo-grid")) return;
        const now = pickedIds(locked);
        if (now.length === 0) return;
        e.preventDefault();
        onAdd(now);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [locked, onAdd, onCancel]);
  return (
    <div className="library-route__pickbar" role="region" aria-label="Add photos">
      <span className="library-route__pick-to">
        Adding to:{" "}
        {album ? <PathName name={album.name} path={albumPath(collections, album)} /> : <span>the album</span>}
      </span>
      <span className="library-route__pick-count">{picked.length ? `${picked.length} selected` : "Click photos to select them"}</span>
      <Button variant="default" size="xs" onClick={onCancel}>
        Cancel
      </Button>
      <Button size="xs" disabled={picked.length === 0} onClick={() => onAdd(picked)}>
        {picked.length === 1 ? "Add 1 photo" : picked.length ? `Add ${picked.length} photos` : "Add photos"}
      </Button>
    </div>
  );
}

type OpenSet = ReturnType<typeof useOpenSet>;

const ROW_INDICATOR = { type: "spring", stiffness: 420, damping: 38, mass: 0.9 } as const;

function Node({ active, icon, label, count, onClick, open }: { active?: boolean; icon: React.ReactNode; label: React.ReactNode; count?: number; onClick: () => void; open?: boolean }) {
  return (
    <button type="button" className="library-route__pick-node" data-active={active || undefined} aria-current={active ? "true" : undefined} aria-expanded={open} onClick={onClick}>
      {active && <motion.span layoutId="pick-indicator" className="library-route__pick-indicator" aria-hidden transition={ROW_INDICATOR} />}
      {open !== undefined ? <IconChevronRight size={14} className="library-route__pick-chevron" data-open={open || undefined} aria-hidden /> : <span className="library-route__pick-chevron" aria-hidden />}
      <span className="library-route__pick-icon">{icon}</span>
      <span className="library-route__pick-label">{label}</span>
      {count !== undefined && <span className="library-route__pick-n">{count.toLocaleString()}</span>}
    </button>
  );
}

/** A head that folds its rows; the rows hang off a faint guide line. */
function Group({ id, title, icon, openSet, children }: { id: string; title: string; icon: React.ReactNode; openSet: OpenSet; children?: React.ReactNode }) {
  const open = openSet.has(id);
  return (
    <div className="library-route__pick-group">
      <Node label={title} icon={icon} open={open} onClick={() => openSet.toggle(id)} />
      <Fold open={open} className="library-route__pick-children">
        {children}
      </Fold>
    </div>
  );
}

const faceOf = (kind: "face" | "pet", id: number | null, fallback: React.ReactNode) =>
  id != null ? <img className="library-route__pick-face" src={mediaUrl(kind, id)} alt="" loading="lazy" draggable={false} /> : fallback;

const coverOf = (style: CoverStyle | null, fallback: React.ReactNode) =>
  style ? (
    <span className="library-route__pick-cover">
      <IconCover style={style} />
    </span>
  ) : (
    fallback
  );

/** Collections, nested, with their albums as the leaves. */
function CollectionNodes({ parent, collections, albums, skip, source, onSource, openSet }: { parent: number | null; collections: AlbumCollection[]; albums: AlbumSummary[]; skip: number; source: PickSource; onSource: (s: PickSource) => void; openSet: OpenSet }) {
  const here = contentsOf(collections, albums, parent);
  return (
    <>
      {here.collections.map((c) => {
        const id = `c:${c.id}`;
        const open = openSet.has(id);
        return (
          <div key={id}>
            <Node label={c.name} icon={coverOf(c.coverStyle, <IconFolder size={16} />)} open={open} onClick={() => openSet.toggle(id)} />
            <Fold open={open} className="library-route__pick-children">
              <CollectionNodes parent={c.id} collections={collections} albums={albums} skip={skip} source={source} onSource={onSource} openSet={openSet} />
            </Fold>
          </div>
        );
      })}
      {here.albums
        .filter((a) => a.kind !== 1 && a.id !== skip)
        .map((a) => {
          const s: PickSource = { kind: "album", id: a.id };
          return <Node key={`a${a.id}`} label={a.name} icon={coverOf(a.coverStyle, <IconAlbum size={16} />)} count={a.itemCount} active={same(source, s)} onClick={() => onSource(s)} />;
        })}
    </>
  );
}

/** A place is typed, not listed: the search reads it like any place in the Library's own box. */
function PlaceNodes({ source, onSource }: { source: PickSource; onSource: (s: PickSource) => void }) {
  const [text, setText] = useState("");
  const [miss, setMiss] = useState(false);
  const find = async () => {
    const q = text.trim();
    if (!q) return;
    const found = await invoke<SearchOverview>("search_overview", { q }).catch(() => null);
    const hit = found?.places[0];
    setMiss(!hit);
    if (hit) onSource({ kind: "place", place: hit.place, label: hit.label });
  };
  return (
    <div className="library-route__pick-place">
      <input
        className="library-route__pick-input"
        aria-label="A place"
        placeholder="Type a place, then Enter"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setMiss(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void find();
          }
        }}
      />
      {miss && <span className="library-route__pick-miss">No place with photos by that name.</span>}
      {source.kind === "place" && <Node label={source.label} active icon={<IconMapPin size={16} />} onClick={() => onSource(source)} />}
    </div>
  );
}

/** Library, Favorites, collections › albums, People, Pets and Places: only scopes the grid already has. */
export function PickTree({ albumId, source, onSource }: { albumId: number; source: PickSource; onSource: (s: PickSource) => void }) {
  const { albums } = useAlbums();
  const { collections } = useAlbumCollections();
  const { people } = usePeople();
  const { pets } = usePets();
  // One set for the whole tree, so a folded branch remembers what was open inside it. Albums start open.
  const openSet = useOpenSet("lumen.pick-tree.open", () => ["g:albums"]);
  const named = people.filter((p) => p.name && p.photoCount > 0);
  const shownPets = pets.filter((p) => p.photoCount > 0);
  return (
    <nav className="library-route__pick-tree" aria-label="Where to find photos">
      <Node label="Library" icon={<IconPhoto size={16} />} active={same(source, LIBRARY_SOURCE)} onClick={() => onSource(LIBRARY_SOURCE)} />
      <Node label="Favorites" icon={<IconStar size={16} />} active={source.kind === "favorites"} onClick={() => onSource({ kind: "favorites" })} />
      <Group id="g:albums" title="Albums" icon={<IconAlbum size={16} />} openSet={openSet}>
        <CollectionNodes parent={null} collections={collections} albums={albums} skip={albumId} source={source} onSource={onSource} openSet={openSet} />
      </Group>
      <Group id="g:people" title="People" icon={<IconUsers size={16} />} openSet={openSet}>
        {named.length === 0 && <span className="library-route__pick-miss">No named people yet.</span>}
        {named.map((p) => {
          const s: PickSource = { kind: "person", id: p.id };
          return <Node key={p.id} label={p.name} icon={faceOf("face", p.coverFaceId, <IconUser size={16} />)} count={p.photoCount} active={same(source, s)} onClick={() => onSource(s)} />;
        })}
      </Group>
      <Group id="g:pets" title="Pets" icon={<IconPaw size={16} />} openSet={openSet}>
        {shownPets.length === 0 && <span className="library-route__pick-miss">No pets found yet.</span>}
        {shownPets.map((p) => {
          const s: PickSource = { kind: "pet", id: p.id };
          return <Node key={p.id} label={petName(p)} icon={faceOf("pet", p.coverDetectionId, <IconPaw size={16} />)} count={p.photoCount} active={same(source, s)} onClick={() => onSource(s)} />;
        })}
      </Group>
      <Group id="g:places" title="Places" icon={<IconMapPin size={16} />} openSet={openSet}>
        <PlaceNodes source={source} onSource={onSource} />
      </Group>
    </nav>
  );
}

/** The album being added to, from the pages that already loaded it. */
export function usePickAlbum(albumId: number) {
  return useQuery({ queryKey: queryKeys.album(albumId), queryFn: () => invoke<AlbumSummary>("get_album", { id: albumId }) });
}
