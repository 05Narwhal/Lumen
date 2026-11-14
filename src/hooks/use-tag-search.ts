import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import { useSearchParams } from "react-router";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { addTag, appendSeq, EMPTY_SEQ, flatTags, fromLegacy, groupPair, type Path, removeAt, removeLast, repair, setJoin, toggleNot, ungroup } from "@/lib/search-expr";
import { parseTags, writeTags } from "@/lib/search-tags";
import type { Join } from "@/types/generated/Join";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { SentenceReading } from "@/types/generated/SentenceReading";
import type { TagSeq } from "@/types/generated/TagSeq";
import type { TagSuggestion } from "@/types/generated/TagSuggestion";

/** The sequence of search tags and every edit of it (Round 20 §3); `tags` is its leaves in reading order. */
export type TagStore = {
  seq: TagSeq;
  tags: SearchTag[];
  add(tag: SearchTag, how?: { join: Join; not: boolean }): void;
  append(seq: TagSeq, lead: Join | null): void;
  removeAt(path: Path): void;
  setJoin(group: Path, index: number, join: Join, not: boolean): void;
  toggleNot(path: Path): void;
  groupPair(group: Path, index: number): void;
  ungroup(path: Path): void;
  removeLast(): void;
  clear(): void;
};

/** The store over `seq`, each edit handed to `update` as a function of the sequence. */
function makeStore(seq: TagSeq, update: (next: (s: TagSeq) => TagSeq) => void): TagStore {
  return {
    seq,
    tags: flatTags(seq),
    add: (tag, how) => update((s) => addTag(s, tag, how)),
    append: (more, lead) => update((s) => appendSeq(s, more, lead)),
    removeAt: (path) => update((s) => removeAt(s, path)),
    setJoin: (group, index, join, not) => update((s) => setJoin(s, group, index, join, not)),
    toggleNot: (path) => update((s) => toggleNot(s, path)),
    groupPair: (group, index) => update((s) => groupPair(s, group, index)),
    ungroup: (path) => update((s) => ungroup(s, path)),
    removeLast: () => update((s) => removeLast(s)),
    clear: () => update(() => EMPTY_SEQ),
  };
}

/**
 * The search tags of the current page (Round 7, a sequence since Round 20), kept in the URL.
 * Changes replace the history entry: Back leaves the page, not one tag.
 */
export function useTagSearch(): TagStore {
  const [params, setParams] = useSearchParams();
  const raw = params.get("tags");
  const legacy = params.get("favorites");
  // Parsed from the parameters that hold them, so a new `asset=` keeps the same sequence.
  const seq = useMemo(() => {
    const only = new URLSearchParams();
    if (raw) only.set("tags", raw);
    if (legacy) only.set("favorites", legacy);
    return parseTags(only);
  }, [raw, legacy]);
  const update = useCallback(
    (next: (s: TagSeq) => TagSeq) =>
      setParams(
        (p) => {
          writeTags(p, next(parseTags(p)));
          return p;
        },
        { replace: true },
      ),
    [setParams],
  );
  return useMemo(() => makeStore(seq, update), [seq, update]);
}

/** The same store over local state: the smart album editor's search. */
export function useSeqStore(initial: TagSeq): TagStore {
  const [seq, setSeq] = useState(() => repair(initial));
  const update = useCallback((next: (s: TagSeq) => TagSeq) => setSeq((s) => next(s)), []);
  return useMemo(() => makeStore(seq, update), [seq, update]);
}

/**
 * `scope` narrowed by `seq`; the same scope when it is empty. A query that already has tags
 * of its own (a smart album) keeps them: the page's sequence is anded to them.
 */
export function withTags(scope: LayoutScope, seq: TagSeq): LayoutScope {
  if (!seq.items.length) return scope;
  const base = scope.query ?? EMPTY_QUERY;
  const own = base.tagExpr ?? fromLegacy(base.searchTags);
  return { ...scope, query: { ...base, tagExpr: appendSeq(own, seq, "and"), searchTags: [] } };
}

/** What the search field offers for `input`. The last list stays up while the next loads. */
/** What the tag search offers for `input`; with `starters`, an empty box asks too (Round 15). */
export function useTagSuggestions(input: string, starters = false) {
  const trimmed = input.trim();
  return useQuery({
    queryKey: queryKeys.tagSuggestions(trimmed),
    queryFn: () => invoke<TagSuggestion[]>("suggest_tags", { input: trimmed }),
    enabled: trimmed.length > 0 || starters,
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
}

/** What a typed sentence reads as (`live and hdr not screenshot`); `null` for one plain term. Asked only when `ask` says the text looks like a sentence. */
export function useSentenceReading(input: string, ask: boolean) {
  const trimmed = input.trim();
  return useQuery({
    queryKey: queryKeys.searchSentence(trimmed),
    queryFn: () => invoke<SentenceReading | null>("read_search_sentence", { input: trimmed }),
    enabled: ask && trimmed.length > 0,
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
}
