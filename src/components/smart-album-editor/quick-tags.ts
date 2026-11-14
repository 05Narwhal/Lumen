// The quick-add strip of the smart album editor (Round 20 §4): tags for people who would
// rather click than type. The labels are copied from Rust's fixed entries (`tag_search.rs`)
// so a tag added here is the same tag the search field would add (`sameTag` compares values).

import type { AssetKind } from "@/types/generated/AssetKind";
import type { SearchTag } from "@/types/generated/SearchTag";

const kind = (label: string, k: AssetKind): SearchTag => ({ label, value: { type: "kind", kind: k } });

export const KIND_TAGS: SearchTag[] = [
  { label: "Photos", value: { type: "media", media: 0 } },
  { label: "Videos", value: { type: "media", media: 1 } },
  kind("Live Photos", "live"),
  kind("RAW", "raw"),
  kind("HDR", "hdr"),
  kind("Screenshots", "screenshot"),
  kind("Panoramas", "panorama"),
  kind("Selfies", "selfie"),
];

export const RATING_TAGS: SearchTag[] = [
  { label: "5 stars", value: { type: "rating", min: 5 } },
  { label: "4 stars and up", value: { type: "rating", min: 4 } },
  { label: "3 stars and up", value: { type: "rating", min: 3 } },
  { label: "2 stars and up", value: { type: "rating", min: 2 } },
  { label: "1 star and up", value: { type: "rating", min: 1 } },
];

export const FLAG_TAGS: SearchTag[] = [
  { label: "Picked", value: { type: "flag", flag: 1 } },
  { label: "Rejected", value: { type: "flag", flag: -1 } },
];

/** Dates are Rust's: each phrase goes through `suggest_tags` and the first *Date* suggestion is the tag. */
export const DATE_PHRASES = ["This year", "Last year", "Last 30 days", "Last 12 months"];
