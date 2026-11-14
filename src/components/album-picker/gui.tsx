import { Button, Modal, TextInput } from "@mantine/core";
import { IconAlbum, IconChevronRight, IconFolder, IconPlus, IconSearch } from "@tabler/icons-react";
import { useEffect, useMemo, useState, type CSSProperties, type KeyboardEvent } from "react";
import { Fold } from "@/components/fold";
import { IconCover } from "@/components/icon-cover";
import { Spinner } from "@/components/spinner";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { clearIfSelected } from "@/hooks/use-selection";
import { albumPath } from "@/lib/album-collections";
import { readRecentAlbumIds, recentAlbums } from "@/lib/album-recent";
import { searchAlbums } from "@/lib/album-search";
import { buildAlbumTree, countRows, visibleRows, type AlbumTreeNode } from "@/lib/album-tree";
import { useOpenSet } from "@/lib/open-set";
import { mediaUrl } from "@/utils/media";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { albumPickerStore, closeAlbumPicker } from "./store";
import "./styles.scss";

/** With this many rows or fewer the tree starts fully open; more, and the person opens what they need. */
const OPEN_ALL_UNTIL = 12;
/** The recent albums are worth a section only when there is something to choose between. */
const RECENT_FROM_ALBUMS = 6;

/** One thing the arrow keys can land on. `parent` is the key of the collection it sits in. */
type NavRow = {
  key: string;
  parent: string | null;
  album?: AlbumSummary;
  collection?: AlbumCollection;
  create?: true;
};

const rowId = (key: string) => `album-picker-row-${key}`;
const collectionIds = (nodes: AlbumTreeNode[]): string[] =>
  nodes.flatMap((n) => (n.kind === "collection" ? [String(n.collection.id), ...collectionIds(n.children)] : []));
const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

function Cover({ album }: { album: AlbumSummary }) {
  return (
    <span className="album-picker__cover">
      {album.coverStyle ? (
        <IconCover style={album.coverStyle} />
      ) : album.coverAssetId ? (
        <img src={mediaUrl("thumb", album.coverAssetId, { s: 256 })} alt="" />
      ) : (
        <IconAlbum size={16} />
      )}
    </span>
  );
}

type RowUi = {
  active: string | null;
  setActive: (key: string) => void;
  pick: (album: AlbumSummary) => void;
};

/** An album row: cover, name (with a dim `after` on the same line, or `below` on a second one), count. */
function AlbumRow({ rowKey, album, depth = 0, after, below, ui }: { rowKey: string; album: AlbumSummary; depth?: number; after?: string; below?: string; ui: RowUi }) {
  return (
    <div
      id={rowId(rowKey)}
      role="treeitem"
      aria-selected={ui.active === rowKey}
      aria-level={depth + 1}
      className="album-picker__row"
      data-active={ui.active === rowKey || undefined}
      style={{ "--depth": depth } as CSSProperties}
      onMouseMove={() => ui.active !== rowKey && ui.setActive(rowKey)}
      onClick={() => ui.pick(album)}
    >
      <Cover album={album} />
      <span className="album-picker__text">
        <span className="album-picker__name">
          {album.name}
          {after && <span className="album-picker__after"> · {after}</span>}
        </span>
        {below && <span className="album-picker__path">{below}</span>}
      </span>
      <span className="album-picker__count">{album.itemCount.toLocaleString()}</span>
    </div>
  );
}

function Nodes({ nodes, depth, ui, isOpen, toggle }: { nodes: AlbumTreeNode[]; depth: number; ui: RowUi; isOpen: (id: number) => boolean; toggle: (id: number) => void }) {
  return (
    <>
      {nodes.map((node) => {
        if (node.kind === "album") return <AlbumRow key={node.key} rowKey={node.key} album={node.album} depth={depth} ui={ui} />;
        const { collection } = node;
        const open = isOpen(collection.id);
        return (
          <div key={node.key}>
            <div
              id={rowId(node.key)}
              role="treeitem"
              aria-expanded={open}
              aria-selected={ui.active === node.key}
              aria-level={depth + 1}
              className="album-picker__row album-picker__row--collection"
              data-active={ui.active === node.key || undefined}
              style={{ "--depth": depth } as CSSProperties}
              onMouseMove={() => ui.active !== node.key && ui.setActive(node.key)}
              onClick={() => toggle(collection.id)}
            >
              <IconChevronRight size={14} className="album-picker__chevron" data-open={open || undefined} aria-hidden />
              <span className="album-picker__cover album-picker__cover--folder">
                {collection.coverStyle ? <IconCover style={collection.coverStyle} /> : <IconFolder size={18} />}
              </span>
              <span className="album-picker__text">
                <span className="album-picker__name">{collection.name}</span>
              </span>
              <span className="album-picker__count">{node.albums.toLocaleString()}</span>
            </div>
            <Fold open={open}>
              <Nodes nodes={node.children} depth={depth + 1} ui={ui} isOpen={isOpen} toggle={toggle} />
            </Fold>
          </div>
        );
      })}
    </>
  );
}

/** "Add to album…" for a set of asset ids: search, recent albums and the collection tree. Mount once. */
export function AlbumPickerHost() {
  const { ids } = albumPickerStore.use();
  const { albums, isLoading, create, addTo } = useAlbums();
  const { collections } = useAlbumCollections();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [active, setActive] = useState<string | null>(null);
  const open = ids !== null;

  useEffect(() => {
    if (open) return;
    setQuery("");
    setActive(null);
    setCreating(false);
    setName("");
  }, [open]);

  const ordinary = useMemo(() => albums.filter((a) => a.kind === 0), [albums]);
  const tree = useMemo(() => buildAlbumTree(collections, albums), [collections, albums]);
  const openSet = useOpenSet("lumen.album-picker.open", () => (countRows(tree) <= OPEN_ALL_UNTIL ? collectionIds(tree) : []));
  const isOpen = (id: number) => openSet.has(String(id));
  const searching = query.trim() !== "";

  const hits = useMemo(
    () => (searching ? searchAlbums(query, collections, ordinary).flatMap((h) => (h.kind === "album" ? [h] : [])) : []),
    [searching, query, collections, ordinary],
  );
  const showCreate = searching && !ordinary.some((a) => same(a.name, query));
  // Read when the picker opens (and when the albums change), not on every render.
  const recents = useMemo(
    () => (open && !searching && ordinary.length > RECENT_FROM_ALBUMS ? recentAlbums(readRecentAlbumIds(), albums) : []),
    [open, searching, ordinary.length, albums],
  );

  // Every row the keys can reach, in reading order.
  const nav: NavRow[] = searching
    ? [...hits.map((h) => ({ key: `s${h.album.id}`, parent: null, album: h.album })), ...(showCreate ? [{ key: "create", parent: null, create: true as const }] : [])]
    : [
        ...recents.map((a) => ({ key: `r${a.id}`, parent: null, album: a })),
        ...visibleRows(tree, isOpen).map(({ node, parent }) =>
          node.kind === "album" ? { key: node.key, parent, album: node.album } : { key: node.key, parent, collection: node.collection },
        ),
      ];
  const activeKey = nav.some((r) => r.key === active) ? active : (nav[0]?.key ?? null);
  const activeRow = nav.find((r) => r.key === activeKey);

  const pick = async (albumId: number, albumName: string) => {
    if (!ids) return;
    closeAlbumPicker();
    try {
      await addTo({ albumId, assetIds: ids, albumName });
    } catch {
      // `useAlbums` has said what went wrong; the selection stays for another try.
      return;
    }
    // Cancelling keeps the selection; adding is the end of it.
    clearIfSelected(ids);
  };

  const createAndAdd = async (albumName: string) => {
    if (!ids || !albumName.trim()) return;
    const album = await create(albumName.trim());
    setName("");
    setCreating(false);
    await pick(album.id, album.name);
  };

  const move = (to: string | null | undefined) => {
    if (!to) return;
    setActive(to);
    document.getElementById(rowId(to))?.scrollIntoView?.({ block: "nearest" });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const at = nav.findIndex((r) => r.key === activeKey);
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(nav[Math.min(nav.length - 1, at + 1)]?.key);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(nav[Math.max(0, at - 1)]?.key);
        break;
      case "Enter":
        e.preventDefault();
        if (activeRow?.album) void pick(activeRow.album.id, activeRow.album.name);
        else if (activeRow?.collection) openSet.toggle(String(activeRow.collection.id));
        else if (activeRow?.create) void createAndAdd(query);
        break;
      case "ArrowRight":
      case "ArrowLeft": {
        // With text in the box the arrows belong to the caret.
        if (searching || !activeRow) break;
        e.preventDefault();
        const c = activeRow.collection;
        if (e.key === "ArrowRight") {
          if (c) openSet.set(String(c.id), true);
        } else if (c && isOpen(c.id)) openSet.set(String(c.id), false);
        else move(activeRow.parent);
        break;
      }
    }
  };

  const ui: RowUi = { active: activeKey, setActive, pick: (a) => void pick(a.id, a.name) };
  const recentWhere = (a: AlbumSummary) => albumPath(collections, a).at(-1);

  return (
    <Modal
      opened={open}
      onClose={closeAlbumPicker}
      title={ids && ids.length > 1 ? `Add ${ids.length} items to album` : "Add to album"}
      centered
      size={440}
      classNames={{ content: "album-picker", header: "album-picker__header", body: "album-picker__body" }}
    >
      {isLoading ? (
        <div className="album-picker__loading">
          <Spinner size="md" label="Loading albums" />
        </div>
      ) : (
        <>
          <TextInput
            data-autofocus
            className="album-picker__search"
            placeholder="Search albums…"
            aria-label="Search albums"
            leftSection={<IconSearch size={16} />}
            value={query}
            onChange={(e) => {
              setQuery(e.currentTarget.value);
              setActive(null);
            }}
            onKeyDown={onKeyDown}
            role="combobox"
            aria-expanded
            aria-controls="album-picker-tree"
            aria-autocomplete="list"
            aria-activedescendant={activeKey ? rowId(activeKey) : undefined}
          />
          <div className="album-picker__scroll">
            <div id="album-picker-tree" role="tree" aria-label="Albums" className="album-picker__list">
              {searching ? (
                <>
                  {hits.map((h) => (
                    <AlbumRow key={`s${h.album.id}`} rowKey={`s${h.album.id}`} album={h.album} below={h.path.join(" › ") || undefined} ui={ui} />
                  ))}
                  {showCreate && (
                    <div
                      id={rowId("create")}
                      role="treeitem"
                      aria-selected={activeKey === "create"}
                      className="album-picker__row album-picker__row--create"
                      data-active={activeKey === "create" || undefined}
                      onMouseMove={() => activeKey !== "create" && setActive("create")}
                      onClick={() => void createAndAdd(query)}
                    >
                      <span className="album-picker__cover album-picker__cover--folder">
                        <IconPlus size={16} />
                      </span>
                      <span className="album-picker__text">
                        <span className="album-picker__name">Create album “{query.trim()}”</span>
                      </span>
                    </div>
                  )}
                </>
              ) : ordinary.length === 0 ? (
                <p className="album-picker__empty">No albums yet.</p>
              ) : (
                <>
                  {recents.length > 0 && (
                    <>
                      <h3 className="album-picker__heading">Recent</h3>
                      {recents.map((a) => (
                        <AlbumRow key={`r${a.id}`} rowKey={`r${a.id}`} album={a} after={recentWhere(a)} ui={ui} />
                      ))}
                    </>
                  )}
                  <h3 className="album-picker__heading">All albums</h3>
                  <Nodes nodes={tree} depth={0} ui={ui} isOpen={isOpen} toggle={(id) => openSet.toggle(String(id))} />
                </>
              )}
            </div>
          </div>
        </>
      )}
      {creating ? (
        <form
          className="album-picker__new"
          onSubmit={(e) => {
            e.preventDefault();
            void createAndAdd(name);
          }}
        >
          <TextInput autoFocus placeholder="Album name" aria-label="Album name" value={name} onChange={(e) => setName(e.currentTarget.value)} />
          <Button type="submit" disabled={!name.trim()}>
            Create
          </Button>
        </form>
      ) : (
        <Button variant="subtle" leftSection={<IconPlus size={16} />} onClick={() => setCreating(true)} className="album-picker__add">
          New album…
        </Button>
      )}
    </Modal>
  );
}
