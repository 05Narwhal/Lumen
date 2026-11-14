import { Button, Menu, Modal } from "@mantine/core";
import { IconChevronDown, IconPlus, IconX } from "@tabler/icons-react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { chipsFor } from "@/components/filter-chips/gui";
import { TagBar, TagSearchField } from "@/components/tag-search";
import { useDevices } from "@/hooks/use-devices";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { EMPTY_QUERY, isEmptyQuery } from "@/hooks/use-search";
import { useSeqStore } from "@/hooks/use-tag-search";
import { invoke } from "@/lib/ipc";
import { queryKeys, scopeKey } from "@/lib/query-keys";
import { addTag, fromLegacy } from "@/lib/search-expr";
import { mediaUrl } from "@/utils/media";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagSeq } from "@/types/generated/TagSeq";
import type { TagSuggestion } from "@/types/generated/TagSuggestion";
import { DATE_PHRASES, FLAG_TAGS, KIND_TAGS, RATING_TAGS } from "./quick-tags";
import "./styles.scss";

export interface SmartAlbumDraft {
  id?: number;
  name: string;
  query: AssetQuery;
}

/** How long rules rest before the count and the preview ask again. */
export const RULES_PAUSE_MS = 300;
/** How many photos the preview shows. */
export const PREVIEW_COUNT = 60;

function useRested<T>(value: T, ms = RULES_PAUSE_MS): T {
  const [rested, setRested] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setRested(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return rested;
}

/** What an opened draft says as one sequence: its expression, or its old tags, plus its old text as one text tag. */
function openSeq(q: AssetQuery): TagSeq {
  const seq = q.tagExpr ?? fromLegacy(q.searchTags ?? []);
  const text = (q.text ?? "").trim();
  if (!text) return seq;
  return addTag(seq, { label: text, value: { type: "text", text } }, { join: "and", not: false });
}

/** A small menu of tags that go straight into the search. */
function QuickMenu({ label, tags, onPick }: { label: string; tags: SearchTag[]; onPick: (t: SearchTag) => void }) {
  return (
    <Menu withinPortal position="bottom-start" shadow="md">
      <Menu.Target>
        <button type="button" className="smart-editor__quick">
          <IconPlus size={13} aria-hidden />
          {label}
          <IconChevronDown size={12} aria-hidden />
        </button>
      </Menu.Target>
      <Menu.Dropdown>
        {tags.map((t) => (
          <Menu.Item key={t.label} onClick={() => onPick(t)}>
            {t.label}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

/**
 * Create or edit a smart album (Round 20 §4). One search box with its token row (the Library's
 * own: `and`, `or`, `not`, groups), a quick-add strip, and what the album would hold right
 * now: a count and the newest photos. Saved as the album's `tagExpr`; the old fields an album
 * may still carry show as *Also:* chips and can be removed, never added.
 */
export function SmartAlbumEditor({
  draft,
  onClose,
  onSave,
}: {
  draft: SmartAlbumDraft;
  onClose: () => void;
  onSave: (d: SmartAlbumDraft) => void;
}) {
  const [name, setName] = useState(draft.name);
  // Everything of the draft's query except the three fields the search owns.
  const [extra, setExtra] = useState<AssetQuery>(() => ({ ...EMPTY_QUERY, ...draft.query, text: null, searchTags: [], tagExpr: undefined }));
  const store = useSeqStore(useMemo(() => openSeq({ ...EMPTY_QUERY, ...draft.query }), [draft]));

  const query = useMemo<AssetQuery>(() => ({ ...extra, tagExpr: store.seq.items.length ? store.seq : undefined }), [extra, store.seq]);
  const rested = useRested(query);
  const none = isEmptyQuery(rested);
  const noRules = isEmptyQuery(query);
  const scope: LayoutScope = { ...LIBRARY_SCOPE, query: rested };
  const count = useQuery({
    queryKey: queryKeys.count(scope),
    queryFn: () => invoke<number>("count_scope", { scope }),
    enabled: !none,
    placeholderData: keepPreviousData,
  });
  const preview = useQuery({
    queryKey: ["smart-preview", scopeKey(scope)],
    queryFn: () => invoke<number[]>("scope_ids", { scope, limit: PREVIEW_COUNT }),
    enabled: !none,
    placeholderData: keepPreviousData,
  });

  const { devices } = useDevices();
  const also = chipsFor(extra, (id) => devices.find((d) => d.id === id)?.displayName ?? `Device ${id}`);

  const addDate = async (phrase: string) => {
    const found = await invoke<TagSuggestion[]>("suggest_tags", { input: phrase });
    const hit = found.find((s) => s.field === "Date");
    if (hit) store.add(hit.tag);
  };

  const n = count.data;
  const counting = none ? null : n == null ? "Counting…" : `${n.toLocaleString()} match`;
  const canSave = name.trim().length > 0 && !noRules;
  const save = () => {
    if (canSave) onSave({ id: draft.id, name: name.trim(), query: { ...extra, tagExpr: store.seq, searchTags: [], text: null } });
  };

  return (
    <Modal
      opened
      onClose={onClose}
      centered
      size="min(960px, 92vw)"
      title={
        <div className="smart-editor__title">
          <input
            className="smart-editor__name"
            aria-label="Album name"
            placeholder="Name this album"
            data-autofocus={draft.id ? undefined : true}
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                save();
              }
            }}
          />
          <span className="smart-editor__badge">Smart album</span>
        </div>
      }
      classNames={{ content: "smart-editor", body: "smart-editor__modal-body", title: "smart-editor__title-box" }}
    >
      <form
        className="smart-editor__body"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <div className="smart-editor__rules">
          <TagSearchField store={store} inputId="smart-album-tags" placeholder="Search: person, place, date, camera…" portal />
          <TagBar store={store} />
          {also.length > 0 && (
            <div className="smart-editor__also" role="list" aria-label="Other rules">
              <span className="smart-editor__also-label">Also:</span>
              {also.map((c) => (
                <span key={c.key} role="listitem" className="smart-editor__chip">
                  {c.label}
                  <button type="button" aria-label={`Remove ${c.label}`} onClick={() => setExtra((q) => ({ ...q, ...c.clear }))}>
                    <IconX size={12} aria-hidden />
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="smart-editor__strip">
            <QuickMenu label="Kind" tags={KIND_TAGS} onPick={store.add} />
            <QuickMenu label="Rating" tags={RATING_TAGS} onPick={store.add} />
            <QuickMenu label="Flag" tags={FLAG_TAGS} onPick={store.add} />
            <Menu withinPortal position="bottom-start" shadow="md">
              <Menu.Target>
                <button type="button" className="smart-editor__quick">
                  <IconPlus size={13} aria-hidden />
                  Date
                  <IconChevronDown size={12} aria-hidden />
                </button>
              </Menu.Target>
              <Menu.Dropdown>
                {DATE_PHRASES.map((p) => (
                  <Menu.Item key={p} onClick={() => void addDate(p)}>
                    {p}
                  </Menu.Item>
                ))}
              </Menu.Dropdown>
            </Menu>
            {counting && (
              <span className="smart-editor__count" aria-live="polite">
                {counting}
              </span>
            )}
          </div>
        </div>
        <section className="smart-editor__preview-box" aria-label="Preview">
          {none ? (
            <p className="smart-editor__empty">Add a tag and the photos that match appear here.</p>
          ) : preview.data && preview.data.length === 0 ? (
            <p className="smart-editor__empty">No photo matches these rules.</p>
          ) : (
            <ul className="smart-editor__preview" aria-label="Newest matching photos">
              {(preview.data ?? []).map((id) => (
                <li key={id}>
                  <img src={mediaUrl("thumb", id, { s: 256 })} alt="" loading="lazy" />
                </li>
              ))}
            </ul>
          )}
        </section>
        <div className="smart-editor__actions">
          <Button variant="default" onClick={onClose} type="button">
            Cancel
          </Button>
          <Button type="submit" disabled={!canSave}>
            {draft.id ? "Save" : "Create album"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
