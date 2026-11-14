import { useAlbumCollections } from "@/hooks/use-album-collections";
import { albumPath } from "@/lib/album-collections";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import "./styles.scss";

/** A name with a dimmed *in Trips › Europe* when `path` (the collections it sits in) is not empty. */
export function PathName({ name, path }: { name: string; path: string[] }) {
  return (
    <span className="album-name">
      <span className="album-name__name">{name}</span>
      {path.length > 0 && <span className="album-name__in"> in {path.join(" › ")}</span>}
    </span>
  );
}

/** An album's name, with a dimmed *in Trips › Europe* when it sits in a collection. */
export function AlbumName({ album }: { album: Pick<AlbumSummary, "name" | "collectionId"> }) {
  const { collections } = useAlbumCollections();
  return <PathName name={album.name} path={albumPath(collections, album)} />;
}
