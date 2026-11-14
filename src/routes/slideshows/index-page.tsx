import { ActionIcon, Button, Menu } from "@mantine/core";
import { IconDotsVertical, IconPlus, IconSlideshow, IconSparkles } from "@tabler/icons-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { PhotoPicker } from "@/components/photo-picker";
import { Spinner } from "@/components/spinner";
import { useMemories } from "@/hooks/use-memories";
import { useSlideshowMenu } from "@/hooks/use-library-menus";
import { useSlideshows } from "@/hooks/use-slideshows";
import type { SlideshowSummary } from "@/types/generated/SlideshowSummary";
import { formatDuration, formatRelative } from "@/utils/date";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

function Row({ show, menu }: { show: SlideshowSummary; menu: () => MenuItem[] }) {
  const { open } = useContextMenu();
  return (
    <article className="slideshows-route__row rise-in" onContextMenu={(e) => open(e, menu())}>
      <Link to={`/slideshows/${show.id}`} className="slideshows-route__main">
        <span className="slideshows-route__cover">
          {show.coverAssetId != null ? <img src={mediaUrl("thumb", show.coverAssetId, { s: 1024 })} alt="" loading="lazy" /> : <IconSlideshow size={22} />}
        </span>
        <span className="slideshows-route__text">
          <span className="slideshows-route__name">{show.name}</span>
          <span className="slideshows-route__meta">
            {show.clipCount} {show.clipCount === 1 ? "photo" : "photos"} · {formatDuration(show.durationMs)} · edited {formatRelative(show.updatedAt)}
          </span>
        </span>
      </Link>
      {/* The same menu as a right-click, from a button for whoever does not right-click. */}
      <ActionIcon variant="subtle" color="gray" aria-label={`More for ${show.name}`} onClick={(e) => open(e, menu())}>
        <IconDotsVertical size={18} />
      </ActionIcon>
    </article>
  );
}

/** A menu of the memories a slideshow can be made from, newest first. */
function FromMemory({ onPick }: { onPick: (memoryId: number) => void }) {
  const { memories } = useMemories();
  return (
    <Menu position="bottom-end" withinPortal>
      <Menu.Target>
        <Button variant="default" leftSection={<IconSparkles size={16} />} disabled={memories.length === 0}>
          From memory
        </Button>
      </Menu.Target>
      <Menu.Dropdown>
        {memories.map((m) => (
          <Menu.Item key={m.id} onClick={() => onPick(m.id)}>
            {m.title}
            {m.subtitle ? ` · ${m.subtitle}` : ""}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

/**
 * Starting a slideshow, shared by the two things on the Memories page's Slideshows tab that can
 * do it (the toolbar's buttons and the empty state's): `picker` is the photo chooser to render
 * once, `newSlideshow` opens it, `fromMemory` makes one from a memory. Each opens the editor.
 */
export function useSlideshowStart() {
  const { create, createFromMemory } = useSlideshows();
  const [picking, setPicking] = useState(false);
  const navigate = useNavigate();
  const open = (id: number) => navigate(`/slideshows/${id}`);
  return {
    newSlideshow: () => setPicking(true),
    fromMemory: (memoryId: number) => void createFromMemory(memoryId).then(open),
    picker: picking ? (
      <PhotoPicker
        title="Choose photos"
        confirmLabel="Make slideshow"
        onClose={() => setPicking(false)}
        onPick={(assetIds) => {
          setPicking(false);
          void create({ name: "New slideshow", assetIds }).then(open);
        }}
      />
    ) : null,
  };
}

type Start = Pick<ReturnType<typeof useSlideshowStart>, "newSlideshow" | "fromMemory">;

const newButton = (onClick: () => void) => (
  <Button leftSection={<IconPlus size={16} />} onClick={onClick}>
    New slideshow
  </Button>
);

/** The Slideshows tab's toolbar (UX spec §6.6): the two ways to start a show. */
export function SlideshowsActions({ start }: { start: Start }) {
  return (
    <>
      <FromMemory onPick={start.fromMemory} />
      {newButton(start.newSlideshow)}
    </>
  );
}

/** The Slideshows tab (UX spec §6.6): every show as a row, or the empty state. */
export function SlideshowsBody({ start }: { start: Start }) {
  const { slideshows, isLoading, remove } = useSlideshows();
  const slideshowMenuFor = useSlideshowMenu(remove);
  if (isLoading) {
    return (
      <div className="slideshows-route__loading">
        <Spinner size="md" label="Loading slideshows" />
      </div>
    );
  }
  if (slideshows.length === 0) {
    return (
      <EmptyState
        icon={<IconSlideshow />}
        title="No slideshows yet"
        description="Turn a set of photos into a video with music and motion."
        action={newButton(start.newSlideshow)}
      />
    );
  }
  return (
    <div className="slideshows-route__list">
      {slideshows.map((s) => (
        <Row key={s.id} show={s} menu={() => slideshowMenuFor(s)} />
      ))}
    </div>
  );
}
