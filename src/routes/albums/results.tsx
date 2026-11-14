import { IconAlbum, IconFilter, IconFolder, IconSearch } from "@tabler/icons-react";
import { Link } from "react-router";
import { PathName } from "@/components/album-name";
import { CoverStack, photoItems } from "@/components/cover-stack";
import { EmptyState } from "@/components/empty-state";
import { IconCover } from "@/components/icon-cover";
import { albumCount, collectionCovers } from "@/lib/album-collections";
import type { AlbumHit } from "@/lib/album-search";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";

/** Where a hit opens. */
export function hitLink(hit: AlbumHit): string {
  return hit.kind === "album" ? `/albums/${hit.album.id}` : `/albums?c=${hit.collection.id}`;
}

/** The photo search for the same words: the last row, and what Enter does when no name matches. */
export function photoSearchLink(text: string): string {
  return `/search?q=${encodeURIComponent(text.trim())}`;
}

function Cover({ hit, folders, albums }: { hit: AlbumHit; folders: AlbumCollection[]; albums: AlbumSummary[] }) {
  if (hit.kind === "album") {
    const a = hit.album;
    return (
      <span className="albums-route__cover" data-look={a.coverStyle ? "icon" : a.previewIds.length > 0 ? "prints" : undefined}>
        {a.coverStyle ? <IconCover style={a.coverStyle} /> : a.previewIds.length > 0 ? <CoverStack items={photoItems(a.previewIds)} /> : <IconAlbum size={20} />}
      </span>
    );
  }
  const c = hit.collection;
  const covers = collectionCovers(folders, albums, c.id);
  return (
    <span className="albums-route__cover" data-look={c.coverStyle ? "icon" : covers.length > 0 ? "prints" : undefined}>
      {c.coverStyle ? <IconCover style={c.coverStyle} /> : covers.length > 0 ? <CoverStack items={covers} /> : <IconFolder size={20} />}
    </span>
  );
}

/**
 * What the Albums page shows while its search box has text (Round 19 §9): the albums and
 * collections whose names match, each with its collection path and cover, then a row that runs
 * the normal photo search. The first hit is the one Enter opens.
 */
export function AlbumSearchResults({
  text,
  hits,
  folders,
  albums,
}: {
  text: string;
  hits: AlbumHit[];
  folders: AlbumCollection[];
  albums: AlbumSummary[];
}) {
  const photos = (
    <Link to={photoSearchLink(text)} className="albums-route__hit albums-route__hit--photos" data-testid="search-photos">
      <span className="albums-route__hit-icon" aria-hidden>
        <IconSearch size={18} />
      </span>
      <span className="albums-route__hit-text">Search photos for “{text.trim()}”</span>
    </Link>
  );
  return (
    <div className={`albums-route__results${hits.length === 0 ? " albums-route__results--empty" : ""}`} role="region" aria-label="Matching albums">
      {hits.length === 0 ? (
        <EmptyState icon={<IconAlbum />} title="No album or collection by that name" description="Search your photos for the same words instead." action={photos} />
      ) : (
        <>
          <ul className="albums-route__hits">
            {hits.map((hit) => {
              const key = hit.kind === "album" ? `a${hit.album.id}` : `c${hit.collection.id}`;
              const name = hit.kind === "album" ? hit.album.name : hit.collection.name;
              const meta =
                hit.kind === "album"
                  ? `${hit.album.itemCount} items${hit.album.kind === 1 ? " · smart album" : ""}`
                  : `${albumCount(folders, albums, hit.collection.id)} albums`;
              return (
                <li key={key}>
                  <Link to={hitLink(hit)} className="albums-route__hit" data-kind={hit.kind}>
                    <Cover hit={hit} folders={folders} albums={albums} />
                    <span className="albums-route__hit-text">
                      <PathName name={name} path={hit.path} />
                      <span className="albums-route__count">
                        {hit.kind === "album" && hit.album.kind === 1 && <IconFilter size={12} aria-hidden />} {meta}
                      </span>
                    </span>
                  </Link>
                </li>
              );
            })}
          </ul>
          {photos}
        </>
      )}
    </div>
  );
}
