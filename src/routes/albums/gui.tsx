import { Button, Modal, Switch, TextInput } from "@mantine/core";
import { IconAlbum, IconChevronRight, IconEyeOff, IconFilter, IconFolder, IconFolderPlus, IconPhoto, IconPlus } from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { useContextMenu } from "@/components/context-menu";
import { CoverStack, photoItems, type CoverItem } from "@/components/cover-stack";
import { IconCover } from "@/components/icon-cover";
import { setMenu, type MenuItem } from "@/lib/menus";
import { useAlbumMenu, useCollectionMenu, useSetActions } from "@/hooks/use-library-menus";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { promptName } from "@/components/confirm-dialog";
import { albumCount, collectionCovers, collectionPath, contentsOf } from "@/lib/album-collections";
import { searchAlbums } from "@/lib/album-search";
import { AlbumSearchResults, hitLink, photoSearchLink } from "./results";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import { EmptyState } from "@/components/empty-state";
import { PageHeader, SearchInput } from "@/components/page-header";
import { SmartAlbumEditor, type SmartAlbumDraft } from "@/components/smart-album-editor";
import { Spinner } from "@/components/spinner";
import { SplitButton } from "@/components/split-button";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { useAlbums } from "@/hooks/use-albums";
import { useCollections } from "@/hooks/use-collections";
import { collectionLink, collectionMeta } from "@/lib/collections";
import type { Collection } from "@/types/generated/Collection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { mediaUrl } from "@/utils/media";
import { AlbumDetail } from "./detail";
import { CollectionPhotos } from "./collection-photos";
import "./styles.scss";

/** A media type (a built-in collection): the Library filtered to one kind, with its newest item as cover. */
function MediaTypeCard({ collection }: { collection: Collection }) {
  const meta = collectionMeta(collection.id);
  const Icon = meta.icon;
  const n = collection.count;
  const { open } = useContextMenu();
  const navigate = useNavigate();
  const { playScope, exportScope } = useSetActions();
  const to = collectionLink(collection.query);
  const scope = { ...LIBRARY_SCOPE, query: collection.query };
  const menu = () =>
    setMenu({
      open: () => navigate(to),
      slideshow: n > 0 ? () => void playScope(scope) : undefined,
      export: n > 0 ? () => void exportScope(scope) : undefined,
    });
  return (
    <Link to={to} className="albums-route__card rise-in" data-collection={collection.id} onContextMenu={(e) => open(e, menu())}>
      <span className="albums-route__cover">
        {collection.coverAssetId != null ? (
          <img src={mediaUrl("thumb", collection.coverAssetId, { s: 1024 })} alt="" loading="lazy" />
        ) : (
          <Icon size={28} />
        )}
        <span className="albums-route__kind" aria-hidden>
          <Icon size={13} />
        </span>
      </span>
      <span className="albums-route__name">{meta.label}</span>
      <span className="albums-route__count">{n === 0 ? "Everything you star" : `${n.toLocaleString()} ${n === 1 ? "item" : "items"}`}</span>
    </Link>
  );
}

/** What a card carries when dragged onto a collection (Round 5A). */
const DRAG_ALBUM = "application/x-lumen-album";
const DRAG_COLLECTION = "application/x-lumen-collection";

function AlbumCard({ album, menu }: { album: AlbumSummary; menu: () => MenuItem[] }) {
  const { open } = useContextMenu();
  const smart = album.kind === 1;
  return (
    <Link
      to={`/albums/${album.id}`}
      className="albums-route__card rise-in"
      onContextMenu={(e) => open(e, menu())}
      draggable
      onDragStart={(e) => e.dataTransfer.setData(DRAG_ALBUM, String(album.id))}
    >
      <span className="albums-route__cover" data-look={album.coverStyle ? "icon" : album.previewIds.length > 0 ? "prints" : undefined}>
        {album.coverStyle ? (
          <IconCover style={album.coverStyle} />
        ) : album.previewIds.length > 0 ? (
          <CoverStack items={photoItems(album.previewIds)} />
        ) : (
          <IconAlbum size={28} />
        )}
        {smart && (
          <span className="albums-route__smart" title="Smart album">
            <IconFilter size={13} />
          </span>
        )}
        {album.hidesItems && (
          <span className="albums-route__smart" title="Hidden from the Library">
            <IconEyeOff size={13} />
          </span>
        )}
      </span>
      <span className="albums-route__name">{album.name}</span>
      <span className="albums-route__count">{album.itemCount} items</span>
    </Link>
  );
}

/**
 * A collection (Round 5A): its albums' covers fanned as prints (or its icon cover), opening into it. An album or another
 * collection dropped on it moves inside.
 */
function AlbumCollectionCard({
  collection,
  covers,
  albums,
  menu,
  onDropAlbum,
  onDropCollection,
}: {
  collection: AlbumCollection;
  covers: CoverItem[];
  albums: number;
  menu: () => MenuItem[];
  onDropAlbum: (id: number) => void;
  onDropCollection: (id: number) => void;
}) {
  const { open } = useContextMenu();
  const [over, setOver] = useState(false);
  const accepts = (e: React.DragEvent) => e.dataTransfer.types.includes(DRAG_ALBUM) || e.dataTransfer.types.includes(DRAG_COLLECTION);
  return (
    <Link
      to={`/albums?c=${collection.id}`}
      className="albums-route__card rise-in"
      data-over={over || undefined}
      onContextMenu={(e) => open(e, menu())}
      draggable
      onDragStart={(e) => e.dataTransfer.setData(DRAG_COLLECTION, String(collection.id))}
      onDragOver={(e) => {
        if (!accepts(e)) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        const album = Number(e.dataTransfer.getData(DRAG_ALBUM));
        const other = Number(e.dataTransfer.getData(DRAG_COLLECTION));
        if (album) onDropAlbum(album);
        else if (other && other !== collection.id) onDropCollection(other);
        e.preventDefault();
      }}
    >
      <span className="albums-route__cover" data-look={collection.coverStyle ? "icon" : covers.length > 0 ? "prints" : undefined}>
        {collection.coverStyle ? <IconCover style={collection.coverStyle} /> : covers.length > 0 ? <CoverStack items={covers} /> : <IconFolder size={28} />}
        <span className="albums-route__kind" aria-hidden>
          <IconFolder size={13} />
        </span>
      </span>
      <span className="albums-route__name">{collection.name}</span>
      <span className="albums-route__count">{albums === 1 ? "1 album" : `${albums} albums`}</span>
    </Link>
  );
}

function NameModal({
  title,
  initial,
  opened,
  onClose,
  onSubmit,
  offerHiding = false,
}: {
  title: string;
  initial: string;
  opened: boolean;
  onClose: () => void;
  onSubmit: (name: string, hides: boolean) => void;
  /** A new album can be one whose photos leave the rest of the library. */
  offerHiding?: boolean;
}) {
  const [name, setName] = useState(initial);
  const [hides, setHides] = useState(false);
  return (
    <Modal opened={opened} onClose={onClose} title={title} centered size="sm" classNames={{ content: "albums-route__modal" }}>
      <form
        className="albums-route__form"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) onSubmit(name.trim(), hides);
        }}
      >
        <TextInput
          autoFocus
          data-autofocus
          label="Album name"
          placeholder="Summer in Lisbon"
          value={name}
          onChange={(e) => setName(e.currentTarget.value)}
        />
        {offerHiding && (
          <label className="albums-route__option" data-on={hides}>
            <span className="albums-route__option-icon" aria-hidden>
              <IconEyeOff size={16} />
            </span>
            <span className="albums-route__option-text">
              <span className="albums-route__option-label">Hide these photos from the Library</span>
              <span className="albums-route__option-hint">Only this album shows them: not the Library, search, the map or memories.</span>
            </span>
            <Switch aria-label="Hide these photos from the Library" checked={hides} onChange={(e) => setHides(e.currentTarget.checked)} />
          </label>
        )}
        <div className="albums-route__form-actions">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!name.trim()}>
            {offerHiding ? "Create" : "Save"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

export function AlbumsRoute() {
  const { id } = useParams();
  if (id) return <AlbumDetail id={Number(id)} />;
  return <AlbumsIndex />;
}

function AlbumsIndex() {
  const { albums, isLoading, create, createSmart, updateSmart } = useAlbums();
  const albumMenus = useAlbumMenu();
  const collectionMenus = useCollectionMenu();
  const { collections } = useCollections();
  const { collections: folders, create: createCollection, moveAlbum, moveCollection } = useAlbumCollections();
  const [params] = useSearchParams();
  const [creating, setCreating] = useState(false);
  const [smartDraft, setSmartDraft] = useState<SmartAlbumDraft | null>(null);
  // Round 19 §9: the top search filters this page by name; Enter opens the first hit.
  const [find, setFind] = useState("");
  const navigate = useNavigate();
  const searching = find.trim() !== "";
  const hits = useMemo(() => (searching ? searchAlbums(find, folders, albums) : []), [searching, find, folders, albums]);

  // `?c=<id>`: the collection open (Round 5A); none is the top, where the media types live.
  const at = params.get("c") ? Number(params.get("c")) : null;
  // Opening a collection from the results ends the search.
  useEffect(() => setFind(""), [at]);
  const path = at == null ? [] : collectionPath(folders, at);
  const here = path.at(-1) ?? null;
  const inside = contentsOf(folders, albums, here?.id ?? null);
  const empty = inside.collections.length === 0 && inside.albums.length === 0;
  // `&all=1`: the collection's photos in one grid, by album (Round 19 §10).
  const showAll = here != null && params.get("all") === "1" && !searching;
  // A new album or smart album made inside a collection goes into it.
  const placeNew = (album: AlbumSummary) => {
    if (here) void moveAlbum({ albumId: album.id, collectionId: here.id, albumName: album.name });
  };
  const newCollection = async () => {
    const name = await promptName({ title: here ? `New collection in “${here.name}”` : "New collection", initial: "", confirmLabel: "Create" });
    if (name) await createCollection({ name, parentId: here?.id ?? null });
  };

  return (
    <div className="albums-route">
      <PageHeader
        title={here ? here.name : "Albums"}
        searchField={
          <SearchInput
            text={find}
            setText={setFind}
            clear={() => setFind("")}
            placeholder="Search albums"
            onEnter={() => searching && navigate(hits.length > 0 ? hitLink(hits[0]) : photoSearchLink(find))}
          />
        }
        back={showAll && here ? { to: `/albums?c=${here.id}`, label: here.name } : here ? { to: path.length > 1 ? `/albums?c=${path[path.length - 2].id}` : "/albums", label: path.length > 1 ? path[path.length - 2].name : "All albums" } : undefined}
        toolbar={
          showAll ? undefined : (
          <>
            {here && (
              <Button variant="default" component={Link} to={`/albums?c=${here.id}&all=1`} leftSection={<IconPhoto size={16} />}>
                All photos
              </Button>
            )}
            <Button variant="default" leftSection={<IconFolderPlus size={16} />} onClick={() => void newCollection()}>
              New collection
            </Button>
            <SplitButton
              label="New album"
              leftSection={<IconPlus size={16} />}
              onClick={() => setCreating(true)}
              menu={[
                { label: "New album", icon: <IconPlus size={16} />, onClick: () => setCreating(true) },
                { label: "New smart album", icon: <IconFilter size={16} />, onClick: () => setSmartDraft({ name: "", query: EMPTY_QUERY }) },
              ]}
            />
          </>
          )
        }
      />
      {/* One level down the back arrow says it all; deeper, the whole way down. */}
      {path.length > 1 && !searching && (
        <nav className="albums-route__path" aria-label="Collection path">
          <Link to="/albums">Albums</Link>
          {path.map((c) => (
            <span key={c.id} className="albums-route__path-step">
              <IconChevronRight size={14} aria-hidden />
              {c.id === here?.id ? <span aria-current="page">{c.name}</span> : <Link to={`/albums?c=${c.id}`}>{c.name}</Link>}
            </span>
          ))}
        </nav>
      )}
      {showAll && here ? <CollectionPhotos collectionId={here.id} name={here.name} /> : null}
      {searching ? (
        <AlbumSearchResults text={find} hits={hits} folders={folders} albums={albums} />
      ) : null}
      {!searching && !here && collections.length > 0 && (
        <section className="albums-route__section" aria-labelledby="albums-media-types">
          <h2 id="albums-media-types" className="albums-route__heading">
            Media types
          </h2>
          <div className="albums-route__grid">
            {collections.map((c) => (
              <MediaTypeCard key={c.id} collection={c} />
            ))}
          </div>
        </section>
      )}
      {!searching && !here && collections.length > 0 && <h2 className="albums-route__heading">My albums</h2>}
      {searching || showAll ? null : isLoading ? (
        <div className="albums-route__loading">
          <Spinner size="md" label="Loading albums" />
        </div>
      ) : empty && here ? (
        <EmptyState
          icon={<IconFolder />}
          title="Nothing in this collection yet"
          description="Drag albums onto a collection, or choose Move to collection… on an album."
          action={<Button onClick={() => setCreating(true)}>New album</Button>}
        />
      ) : empty ? (
        <EmptyState
          icon={<IconAlbum />}
          title="No albums yet"
          description="Albums keep the photos you choose together. A smart album keeps whatever matches its rules."
          action={
            <>
              <Button onClick={() => setCreating(true)}>New album</Button>
              <Button variant="default" onClick={() => setSmartDraft({ name: "", query: EMPTY_QUERY })}>
                New smart album
              </Button>
            </>
          }
        />
      ) : (
        <div className="albums-route__grid">
          {inside.collections.map((c) => (
            <AlbumCollectionCard
              key={`c${c.id}`}
              collection={c}
              covers={collectionCovers(folders, albums, c.id)}
              albums={albumCount(folders, albums, c.id)}
              menu={() => collectionMenus.menuFor(c)}
              onDropAlbum={(id) => {
                const album = albums.find((a) => a.id === id);
                if (album) void moveAlbum({ albumId: id, collectionId: c.id, albumName: album.name });
              }}
              onDropCollection={(id) => {
                const other = folders.find((k) => k.id === id);
                if (other) void moveCollection({ id, into: c.id, name: other.name });
              }}
            />
          ))}
          {inside.albums.map((a) => (
            <AlbumCard
              key={a.id}
              album={a}
              menu={() => albumMenus.menuFor(a, { editRules: () => setSmartDraft({ id: a.id, name: a.name, query: a.query ?? EMPTY_QUERY }) })}
            />
          ))}
        </div>
      )}
      {creating && (
        <NameModal
          title="Create album"
          initial=""
          opened
          offerHiding
          onClose={() => setCreating(false)}
          onSubmit={(name, hides) => {
            setCreating(false);
            void create({ name, hides }).then(placeNew);
          }}
        />
      )}
      {smartDraft && (
        <SmartAlbumEditor
          draft={smartDraft}
          onClose={() => setSmartDraft(null)}
          onSave={(d) => {
            setSmartDraft(null);
            if (d.id) void updateSmart({ id: d.id, name: d.name, query: d.query });
            else void createSmart({ name: d.name, query: d.query }).then(placeNew);
          }}
        />
      )}
      {albumMenus.element}
      {collectionMenus.element}
    </div>
  );
}
