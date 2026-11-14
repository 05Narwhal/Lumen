import { Button } from "@mantine/core";
import { IconAperture } from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PhotoPicker } from "@/components/photo-picker";
import { addToDevelop, developPhotos, developSourceStore, isEmptySource, lastDevelopPhoto } from "@/lib/develop-source";
import { useStripIds } from "@/hooks/use-develop-strip";
import { DevelopWorkspace } from "./workspace";
import "./styles.scss";

/**
 * `/develop` and `/develop/:id` (spec §1). Without an id, the first photo of the filmstrip.
 * Develop starts empty: the user chooses the photos to work on.
 */
export function DevelopRoute() {
  const { id } = useParams();
  const navigate = useNavigate();
  const source = developSourceStore.use();
  const assetId = id && /^\d+$/.test(id) ? Number(id) : null;
  const ids = useStripIds(source);
  // Back to the photo last open when the strip still holds it (after Undo of *Clear
  // filmstrip*, or on coming back to Develop), else the strip's first.
  const last = lastDevelopPhoto.use();
  const first = last !== null && ids.data?.includes(last) ? last : ids.data?.[0];
  useEffect(() => {
    if (assetId === null && first !== undefined) navigate(`/develop/${first}`, { replace: true });
  }, [assetId, first, navigate]);

  useEffect(() => {
    if (assetId !== null) lastDevelopPhoto.set(assetId);
  }, [assetId]);
  const [picking, setPicking] = useState(false);
  // Opened straight on a photo with nothing chosen: the strip holds that photo. Only on
  // arriving at a photo — *Clear filmstrip* empties the source while the URL still names the
  // open photo, and must not have it added straight back.
  useEffect(() => {
    if (assetId !== null && isEmptySource(developSourceStore.get())) addToDevelop([assetId]);
  }, [assetId]);

  if (assetId !== null) return <DevelopWorkspace assetId={assetId} />;
  if (isEmptySource(source)) {
    return (
      <div className="develop develop--empty">
        <EmptyState
          icon={<IconAperture size={40} />}
          title="Choose photos to develop"
          description="Pick the photos you want to work on; they line up in the filmstrip."
          action={<Button onClick={() => setPicking(true)}>Choose photos…</Button>}
        />
        {picking && (
          <PhotoPicker
            title="Choose photos to develop"
            confirmLabel="Develop"
            filters
            onClose={() => setPicking(false)}
            onPick={(picked) => {
              setPicking(false);
              developPhotos(navigate, picked);
            }}
          />
        )}
      </div>
    );
  }
  if (ids.isSuccess && ids.data.length === 0) {
    return (
      <div className="develop develop--empty">
        <EmptyState
          icon={<IconAperture size={40} />}
          title="Nothing to develop here"
          description="Develop works on photos; this set has none to show."
          action={<Button onClick={() => navigate("/library")}>Open the library</Button>}
        />
      </div>
    );
  }
  return <div className="develop" aria-busy />;
}
