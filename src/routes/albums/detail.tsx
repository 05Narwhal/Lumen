import { Button, SegmentedControl } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { IconAlbum, IconDots, IconEyeOff, IconPhotoPlus } from "@tabler/icons-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { openMenuFor, useContextMenu } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { TagBar } from "@/components/tag-search";
import { PhotoGrid } from "@/components/photo-grid";
import { Spinner } from "@/components/spinner";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { clearSelection } from "@/hooks/use-selection";
import { useSettings } from "@/hooks/use-settings";
import { useTagSearch, withTags } from "@/hooks/use-tag-search";
import { invoke } from "@/lib/ipc";
import { pickAlbumUrl } from "@/lib/pick-mode";
import { albumPageMenu } from "@/lib/menus";
import { queryKeys } from "@/lib/query-keys";
import { ROW_HEIGHTS } from "@/routes/library";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { TextEmptyState } from "@/components/text-empty";
import { hasTextTag } from "@/lib/photo-text";
import type { LayoutScope } from "@/types/generated/LayoutScope";

export function AlbumDetail({ id }: { id: number }) {
  const { settings } = useSettings();
  const { rename, removeFrom, setCover, setHides } = useAlbums();
  const { open: openMenu } = useContextMenu();
  const album = useQuery({ queryKey: queryKeys.album(id), queryFn: () => invoke<AlbumSummary>("get_album", { id }) });
  // Back to the collection it is in (Round 5A), else all albums.
  const { collections } = useAlbumCollections();
  const home = album.data?.collectionId != null ? collections.find((c) => c.id === album.data?.collectionId) : undefined;
  const backTo = home ? { to: `/albums?c=${home.id}`, label: home.name } : { to: "/albums", label: "All albums" };
  const [, setSearchParams] = useSearchParams();
  const [byDate, setByDate] = useState(false);
  const navigate = useNavigate();
  // Round 19 §5: adding photos is the Library in pick mode.
  const addPhotos = () => navigate(pickAlbumUrl(id));
  const [editingName, setEditingName] = useState<string | null>(null);
  const tileSize = settings?.appearance.tileSize ?? "m";
  const smart = album.data?.kind === 1;
  // Search tags narrow the album in place (Round 7).
  const { seq, tags } = useTagSearch();
  const scope = useMemo<LayoutScope>(
    () => withTags(smart ? { ...LIBRARY_SCOPE, query: album.data?.query ?? null } : { ...LIBRARY_SCOPE, albumId: id, sortByDate: byDate }, seq),
    [id, byDate, smart, album.data?.query, seq],
  );

  const onOpen = useCallback(
    (assetId: number) =>
      setSearchParams((p) => {
        p.set("asset", String(assetId));
        return p;
      }),
    [setSearchParams],
  );

  const extraMenuItems = useCallback(
    (ids: number[]) => [
      { divider: true as const },
      ...(ids.length === 1 ? [{ label: "Set as album cover", onSelect: () => void setCover({ albumId: id, assetId: ids[0] }) }] : []),
      { label: ids.length === 1 ? "Remove from album" : `Remove ${ids.length} from album`, onSelect: () => void removeFrom({ albumId: id, assetIds: ids }).then(clearSelection) },
    ],
    [id, setCover, removeFrom],
  );

  if (album.isLoading) {
    return (
      <div className="albums-route__loading">
        <Spinner size="md" label="Loading album" />
      </div>
    );
  }
  if (!album.data) {
    return (
      <div className="albums-route">
        <PageHeader title="Album" search={false} />
        <EmptyState icon={<IconAlbum />} title="Album not found" description="It may have been deleted." action={<Button component={Link} to="/albums">All albums</Button>} />
      </div>
    );
  }
  const a = album.data;

  const pageMenu = albumPageMenu(a, { byDate, setByDate, setHides: (hides) => void setHides({ id, hides, name: a.name }) });

  const commitName = () => {
    const name = editingName?.trim();
    setEditingName(null);
    if (name && name !== a.name) void rename({ id, name });
  };

  return (
    <div className="albums-route albums-route--detail">
      <PageHeader
        back={backTo}
        title={
          editingName === null ? (
            <button type="button" className="albums-route__title" onClick={() => setEditingName(a.name)} title="Rename">
              {a.name}
            </button>
          ) : (
            <input
              className="albums-route__title-input"
              aria-label="Album name"
              autoFocus
              value={editingName}
              onChange={(e) => setEditingName(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") setEditingName(null);
              }}
            />
          )
        }
        search="tags"
        toolbar={
          <>
            <span className="albums-route__meta">{a.itemCount} items</span>
            {smart ? (
              <span className="albums-route__meta">Updates itself</span>
            ) : (
              <>
                <div className="albums-route__inline">
                  <SegmentedControl
                    size="xs"
                    aria-label="Order"
                    value={byDate ? "date" : "manual"}
                    onChange={(v) => setByDate(v === "date")}
                    data={[
                      { label: "Manual", value: "manual" },
                      { label: "By date", value: "date" },
                    ]}
                  />
                  <Button
                    size="xs"
                    variant={a.hidesItems ? "light" : "default"}
                    leftSection={<IconEyeOff size={14} />}
                    aria-pressed={a.hidesItems}
                    title={a.hidesItems ? "Its photos appear only here. Click to show them in the Library again." : "Hide its photos from the Library, search, the map and memories"}
                    onClick={() => void setHides({ id, hides: !a.hidesItems, name: a.name })}
                  >
                    Hidden from the Library
                  </Button>
                </div>
                {/* The same two actions when the header is too narrow for them (a container query in styles.scss). */}
                <Button
                  size="xs"
                  variant="default"
                  className="albums-route__fold"
                  aria-label="More actions"
                  aria-haspopup="menu"
                  onClick={(e) => openMenuFor(e.currentTarget)}
                  onContextMenu={(e) => openMenu(e, pageMenu)}
                >
                  <IconDots size={16} />
                </Button>
                <Button size="xs" leftSection={<IconPhotoPlus size={14} />} onClick={addPhotos}>
                  Add photos
                </Button>
              </>
            )}
          </>
        }
      />
      {a.itemCount === 0 ? (
        <EmptyState
          icon={<IconAlbum />}
          title="This album is empty"
          description="Add photos from your library, or drag tiles onto this album in the sidebar."
          action={<Button onClick={addPhotos}>Add photos</Button>}
        />
      ) : (
        <>
          <TagBar />
          <div className="albums-route__grid-host">
            <PhotoGrid
              rowHeight={ROW_HEIGHTS[tileSize]}
              scope={scope}
              onOpen={onOpen}
              extraMenuItems={extraMenuItems}
              empty={hasTextTag(tags) ? <TextEmptyState /> : undefined}
            />
          </div>
        </>
      )}
    </div>
  );
}
