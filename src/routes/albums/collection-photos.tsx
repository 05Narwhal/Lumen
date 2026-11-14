import { Button } from "@mantine/core";
import { IconPhoto } from "@tabler/icons-react";
import { useCallback, useMemo } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PhotoGrid } from "@/components/photo-grid";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { useSettings } from "@/hooks/use-settings";
import { ROW_HEIGHTS } from "@/routes/library";
import type { LayoutScope } from "@/types/generated/LayoutScope";

/**
 * *All photos* of a collection (Round 19 §10): every photo in its albums, nested ones included,
 * in one grid with a section per album. A photo in two albums is in both sections, so the
 * viewer is opened with the section it was clicked in (`?section=`).
 */
export function CollectionPhotos({ collectionId, name }: { collectionId: number; name: string }) {
  const { settings } = useSettings();
  const navigate = useNavigate();
  const [, setSearchParams] = useSearchParams();
  const scope = useMemo<LayoutScope>(() => ({ ...LIBRARY_SCOPE, collectionId }), [collectionId]);
  const onOpen = useCallback(
    (assetId: number, section?: number) =>
      setSearchParams((p) => {
        p.set("asset", String(assetId));
        if (section != null) p.set("section", String(section));
        else p.delete("section");
        return p;
      }),
    [setSearchParams],
  );
  const openAlbum = useCallback((albumId: number) => navigate(`/albums/${albumId}`), [navigate]);
  return (
    <div className="albums-route__grid-host">
      <PhotoGrid
        rowHeight={ROW_HEIGHTS[settings?.appearance.tileSize ?? "m"]}
        scope={scope}
        onOpen={onOpen}
        onOpenSection={openAlbum}
        empty={
          <EmptyState
            icon={<IconPhoto />}
            title={`No photos in ${name} yet`}
            description="Photos show here once an album in this collection has some."
            action={
              <Button component={Link} to={`/albums?c=${collectionId}`}>
                Back to the albums
              </Button>
            }
          />
        }
      />
    </div>
  );
}
