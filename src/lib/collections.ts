// The built-in collections. Rust decides what each one holds (photos-core::collections);
// this names them, picks their icons and turns their query into a Library link — so a
// collection is the Library grid with a filter chip, not a second kind of page.

import {
  IconAperture,
  IconCameraSelfie,
  IconHdr,
  IconLivePhoto,
  IconPanoramaHorizontal,
  IconScreenshot,
  IconStarFilled,
  IconVideo,
  type Icon,
} from "@tabler/icons-react";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { plainTags } from "@/lib/search-expr";
import { FAVORITES_TAG, sameTag } from "@/lib/search-tags";
import type { AssetKind } from "@/types/generated/AssetKind";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { CollectionId } from "@/types/generated/CollectionId";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagSeq } from "@/types/generated/TagSeq";

export interface CollectionMeta {
  id: CollectionId;
  label: string;
  icon: Icon;
  /** The query, mirrored from Rust so a title can be found without a round trip. */
  query: Partial<AssetQuery>;
}

export const KIND_LABELS: Record<AssetKind, string> = {
  live: "Live Photos",
  raw: "RAW",
  hdr: "HDR",
  screenshot: "Screenshots",
  panorama: "Panoramas",
  selfie: "Selfies",
  silent: "No sound",
};

export const COLLECTIONS: CollectionMeta[] = [
  { id: "favorites", label: "Favorites", icon: IconStarFilled, query: { favorites: true } },
  { id: "videos", label: "Videos", icon: IconVideo, query: { mediaType: 1 } },
  { id: "live", label: KIND_LABELS.live, icon: IconLivePhoto, query: { kind: "live" } },
  { id: "raw", label: KIND_LABELS.raw, icon: IconAperture, query: { kind: "raw" } },
  { id: "hdr", label: KIND_LABELS.hdr, icon: IconHdr, query: { kind: "hdr" } },
  { id: "screenshots", label: KIND_LABELS.screenshot, icon: IconScreenshot, query: { kind: "screenshot" } },
  { id: "panoramas", label: KIND_LABELS.panorama, icon: IconPanoramaHorizontal, query: { kind: "panorama" } },
  { id: "selfies", label: KIND_LABELS.selfie, icon: IconCameraSelfie, query: { kind: "selfie" } },
];

export const collectionMeta = (id: CollectionId) => COLLECTIONS.find((c) => c.id === id)!;

/** Only the fields that differ from an empty query, which is what `useSearch` reads back. */
function narrowing(q: AssetQuery): Partial<AssetQuery> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(q)) {
    const empty = (EMPTY_QUERY as unknown as Record<string, unknown>)[k];
    if (JSON.stringify(v) !== JSON.stringify(empty)) out[k] = v;
  }
  return out as Partial<AssetQuery>;
}

const RATING_LABELS = ["", "1 star and up", "2 stars and up", "3 stars and up", "4 stars and up", "5 stars"];

/**
 * A query as search tags (Round 7), when every field it sets has a tag: a collection, or a
 * link from before tags. `null` when it narrows by something tags cannot say (a person by id).
 */
export function queryToTags(query: Partial<AssetQuery>): SearchTag[] | null {
  const q = narrowing({ ...EMPTY_QUERY, ...query });
  const tags: SearchTag[] = [];
  for (const [k, v] of Object.entries(q)) {
    switch (k) {
      case "favorites":
        tags.push(FAVORITES_TAG);
        break;
      case "mediaType":
        tags.push(v === 1 ? { label: "Videos", value: { type: "media", media: 1 } } : { label: "Photos", value: { type: "media", media: 0 } });
        break;
      case "kind":
        tags.push({
          label: KIND_LABELS[v as AssetKind],
          value: { type: "kind", kind: v as AssetKind },
        });
        break;
      case "minRating": {
        const min = Math.min(5, Math.max(1, Number(v)));
        tags.push({
          label: RATING_LABELS[min],
          value: { type: "rating", min },
        });
        break;
      }
      case "flag":
        tags.push(v === 1 ? { label: "Picked", value: { type: "flag", flag: 1 } } : { label: "Rejected", value: { type: "flag", flag: -1 } });
        break;
      case "hasGps":
        tags.push(v ? { label: "Has location", value: { type: "location", has: true } } : { label: "No location", value: { type: "location", has: false } });
        break;
      case "extension": {
        const ext = String(v).replace(/^\./, "").toLowerCase();
        tags.push({
          label: ext.toUpperCase(),
          value: { type: "extension", ext },
        });
        break;
      }
      case "text": {
        const text = String(v).trim();
        if (text) tags.push({ label: text, value: { type: "text", text } });
        break;
      }
      case "searchTags":
        tags.push(...(v as SearchTag[]));
        break;
      case "tagExpr": {
        // A smart album's expression (Round 20): tags can say it only when it is a plain list.
        const plain = plainTags(v as TagSeq);
        if (!plain) return null;
        tags.push(...plain);
        break;
      }
      default:
        return null;
    }
  }
  return tags;
}

/** The Library link of a collection: its tags, or its query where tags cannot say it. */
export function collectionLink(q: AssetQuery): string {
  const tags = queryToTags(q);
  return tags ? `/library?tags=${encodeURIComponent(JSON.stringify(tags))}` : `/library?filters=${encodeURIComponent(JSON.stringify(narrowing(q)))}`;
}

/** The collection a page's tags are exactly, for the Library's title. */
export function collectionForTags(tags: SearchTag[]): CollectionMeta | undefined {
  if (tags.length !== 1) return undefined;
  return COLLECTIONS.find((c) => {
    const own = queryToTags(c.query);
    return own?.length === 1 && sameTag(own[0], tags[0]);
  });
}

/** The collection `q` is exactly, for the Library's title. Search text doesn't count. */
export function collectionFor(q: AssetQuery): CollectionMeta | undefined {
  const n = JSON.stringify(narrowing({ ...q, text: null }));
  return COLLECTIONS.find((c) => JSON.stringify(narrowing({ ...EMPTY_QUERY, ...c.query })) === n);
}
