import { Button } from "@mantine/core";
import { IconArrowLeft, IconClock, IconEye, IconEyeOff, IconMovie, IconPlayerPlayFilled } from "@tabler/icons-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PhotoGrid } from "@/components/photo-grid";
import { Segmented } from "@/components/segmented";
import { TagBar, TagSearchField } from "@/components/tag-search";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { useHideMemory, useMemory } from "@/hooks/use-memories";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { useSettings } from "@/hooks/use-settings";
import { useTagSearch, withTags } from "@/hooks/use-tag-search";
import { exportMemoryFilm, playFilm } from "@/lib/film";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import { TextEmptyState } from "@/components/text-empty";
import { hasTextTag } from "@/lib/photo-text";
import { ROW_HEIGHTS } from "@/routes/library";
import { mediaUrl } from "@/utils/media";

type Show = "highlights" | "all";

/**
 * One memory (UX spec §6.5): its cover and dates, *Play* (its film, Round 4F: music,
 * transitions and cuts on the beat; the film's *Edit slideshow* replaces *Make slideshow*),
 * *Hide*, and a grid of either its highlights (the curated set) or every photo from its period.
 */
export function MemoryDetail({ id }: { id: number }) {
  const memory = useMemory(id);
  const hide = useHideMemory();
  const { settings } = useSettings();
  const [, setParams] = useSearchParams();
  const [show, setShow] = useState<Show>("highlights");
  const m = memory.data;

  // Search tags narrow the memory in place (Round 7).
  const { seq, tags } = useTagSearch();
  const scope = useMemo<LayoutScope>(
    () =>
      withTags(
        show === "highlights" || !m
          ? { ...LIBRARY_SCOPE, memoryId: id }
          : {
              ...LIBRARY_SCOPE,
              query: { ...EMPTY_QUERY, dateFrom: m.startAt, dateTo: m.endAt },
            },
        seq,
      ),
    [show, id, m, seq],
  );

  const onOpen = useCallback(
    (assetId: number) =>
      setParams((p) => {
        p.set("asset", String(assetId));
        return p;
      }),
    [setParams],
  );

  if (memory.isError) {
    return (
      <div className="memories-route">
        <EmptyState
          icon={<IconClock />}
          title="This memory is gone"
          description="Memories change as the library does."
          action={
            <Button component={Link} to="/memories">
              All memories
            </Button>
          }
        />
      </div>
    );
  }

  const tileSize = settings?.appearance.tileSize ?? "m";
  return (
    <div className="memories-route memories-route--detail">
      <header className="memory-hero">
        {m?.coverAssetId != null && <img className="memory-hero__img" src={mediaUrl("preview", m.coverAssetId, { s: 2560 })} alt="" />}
        <div className="memory-hero__scrim" />
        <Button component={Link} to="/memories" variant="subtle" size="xs" className="memory-hero__back" leftSection={<IconArrowLeft size={14} />}>
          Memories
        </Button>
        <div className="memory-hero__text">
          <h1>{m?.title ?? "…"}</h1>
          {m?.subtitle && <p>{m.subtitle}</p>}
        </div>
        {m && (
          <div className="memory-hero__actions">
            <Button leftSection={<IconPlayerPlayFilled size={16} />} onClick={() => playFilm({ memoryId: m.id, title: m.title, coverAssetId: m.coverAssetId })}>
              Play
            </Button>
            <Button variant="default" leftSection={<IconMovie size={16} />} onClick={() => void exportMemoryFilm(m.id)}>
              Export video
            </Button>
            <Button variant="default" leftSection={m.hidden ? <IconEye size={16} /> : <IconEyeOff size={16} />} onClick={() => void hide(m, !m.hidden)}>
              {m.hidden ? "Unhide" : "Hide"}
            </Button>
          </div>
        )}
      </header>
      {m && (
        <div className="memories-route__bar">
          <Segmented<Show>
            label="Show"
            value={show}
            onChange={setShow}
            data={[
              { value: "highlights", label: `Highlights (${m.curatedCount.toLocaleString()})` },
              { value: "all", label: `All photos (${m.itemCount.toLocaleString()})` },
            ]}
          />
          <TagSearchField />
        </div>
      )}
      <TagBar />
      <div className="memories-route__grid-host">
        <PhotoGrid rowHeight={ROW_HEIGHTS[tileSize]} scope={scope} onOpen={onOpen} empty={hasTextTag(tags) ? <TextEmptyState /> : undefined} />
      </div>
    </div>
  );
}
