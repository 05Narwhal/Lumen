import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { parseTags } from "@/lib/search-tags";
import type { AssetQuery } from "@/types/generated/AssetQuery";

/** An `AssetQuery` with nothing set. Kept here so callers never build one by hand. */
export const EMPTY_QUERY: AssetQuery = {
  text: null,
  dateFrom: null,
  dateTo: null,
  deviceIds: [],
  mediaType: null,
  favorites: false,
  minRating: null,
  hasGps: null,
  hasText: false,
  noText: false,
  inText: [],
  camera: null,
  lens: null,
  kind: null,
  extension: null,
  includeRawPairs: false,
  colorLabels: [],
  flag: null,
  personIds: [],
  petIds: [],
  places: [],
  tags: [],
  months: [],
  albumIds: [],
  collectionIds: [],
  anyOf: [],
  searchTags: [],
};

const DEBOUNCE_MS = 200;

/** Whether a query narrows anything, so callers can send `null` instead. */
export function isEmptyQuery(q: AssetQuery): boolean {
  return (
    !q.text?.trim() &&
    q.dateFrom == null &&
    q.dateTo == null &&
    q.deviceIds.length === 0 &&
    q.mediaType == null &&
    !q.favorites &&
    !q.minRating &&
    q.hasGps == null &&
    !q.hasText &&
    !q.noText &&
    !q.inText?.length &&
    !q.camera?.trim() &&
    !q.lens?.trim() &&
    q.kind == null &&
    q.colorLabels.length === 0 &&
    q.flag == null &&
    !q.personIds?.length &&
    !q.petIds?.length &&
    !q.places?.length &&
    !q.tags?.length &&
    !q.months?.length &&
    !q.albumIds?.length &&
    !q.collectionIds?.length &&
    !q.anyOf?.length &&
    !q.searchTags?.length &&
    !q.tagExpr?.items.length
  );
}

/** Structured filters travel as one compact JSON parameter; the text stays readable. */
function parse(params: URLSearchParams): AssetQuery {
  const text = params.get("q");
  let filters: Partial<AssetQuery> = {};
  const raw = params.get("filters");
  if (raw) {
    try {
      filters = JSON.parse(raw) as Partial<AssetQuery>;
    } catch {
      // A hand-edited URL should not break the page.
    }
  }
  const tagExpr = parseTags(params);
  return { ...EMPTY_QUERY, ...filters, text: text?.trim() ? text : null, searchTags: [], tagExpr: tagExpr.items.length ? tagExpr : undefined };
}

/**
 * The search for the current route, stored in the URL so it survives reload, Back and a
 * shared link. Typing is debounced into the URL; the input itself stays instant.
 */
export function useSearch() {
  const [params, setParams] = useSearchParams();
  const query = parse(params);
  const urlText = query.text ?? "";
  const [text, setTextState] = useState(urlText);

  // Back/forward and programmatic navigation win over the local draft.
  useEffect(() => setTextState(urlText), [urlText]);

  useEffect(() => {
    if (text === urlText) return;
    const t = setTimeout(() => {
      setParams(
        (p) => {
          if (text.trim()) p.set("q", text);
          else p.delete("q");
          return p;
        },
        { replace: true },
      );
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [text, urlText, setParams]);

  const setFilters = useCallback(
    (patch: Partial<AssetQuery>) => {
      setParams((p) => {
        const next = { ...parse(p), ...patch };
        // The tags have their own parameter (`tags=`); `filters=` never repeats them.
        const { text: _t, searchTags: _s, tagExpr: _e, ...rest } = next;
        const trimmed = Object.fromEntries(
          Object.entries(rest).filter(([, v]) => v != null && v !== false && !(Array.isArray(v) && v.length === 0)),
        );
        if (Object.keys(trimmed).length) p.set("filters", JSON.stringify(trimmed));
        else p.delete("filters");
        return p;
      });
    },
    [setParams],
  );

  const clear = useCallback(() => {
    setTextState("");
    setParams(
      (p) => {
        p.delete("q");
        p.delete("filters");
        return p;
      },
      { replace: true },
    );
  }, [setParams]);

  return { query, text, setText: setTextState, setFilters, clear, active: !isEmptyQuery(query) };
}
