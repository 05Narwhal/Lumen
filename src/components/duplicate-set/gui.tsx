import { Button, Tooltip } from "@mantine/core";
import { IconCopy, IconHeartFilled, IconPhotoSearch, IconStarFilled } from "@tabler/icons-react";
import type { KeyboardEvent } from "react";
import { toggleQuickLook } from "@/components/quick-look/store";
import type { DuplicateSet } from "@/types/generated/DuplicateSet";
import type { Likeness } from "@/types/generated/Likeness";
import { formatBytes } from "@/utils/format";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** How each kind is named and explained (Round 4D: one list, told apart by a badge). */
export const LIKENESS: Record<Likeness, { label: string; tip: string; icon: typeof IconCopy }> = {
  exact: {
    label: "Exact copy",
    tip: "The same picture stored more than once — perhaps in another format or size. Safe to remove.",
    icon: IconCopy,
  },
  similar: {
    label: "Similar",
    tip: "Taken moments apart and almost identical, but not the same picture. Check before removing.",
    icon: IconPhotoSearch,
  },
};

/**
 * One set of duplicates: its kind, its photos with the one to keep marked (click another to
 * keep it instead; Space looks closer), and what to do with it.
 */
export function DuplicateSetRow({
  set,
  keep,
  onKeep,
  onRemove,
  onDismiss,
}: {
  set: DuplicateSet;
  keep: number;
  onKeep: (id: number) => void;
  onRemove: () => void;
  onDismiss: () => void;
}) {
  const kind = LIKENESS[set.likeness];
  const Icon = kind.icon;
  const others = set.members.length - 1;
  const look = (e: KeyboardEvent, id: number) => {
    if (e.key !== " ") return;
    e.preventDefault();
    toggleQuickLook(id);
  };
  return (
    <section className="duplicate-set" role="group" aria-label={`${kind.label}: ${set.members.length} photos`}>
      <Tooltip label={kind.tip} multiline w={260} openDelay={200} withArrow>
        <span className="duplicate-set__badge" data-likeness={set.likeness} tabIndex={0}>
          <Icon size={14} aria-hidden /> {kind.label}
        </span>
      </Tooltip>
      <div className="duplicate-set__photos">
        {set.members.map((m) => {
          const kept = m.id === keep;
          return (
            <button
              key={m.id}
              type="button"
              className="duplicate-set__photo"
              aria-pressed={kept}
              aria-label={`Photo ${m.id}, ${m.width} × ${m.height} ${m.format}${kept ? ", kept" : ""}`}
              onClick={() => onKeep(m.id)}
              onKeyDown={(e) => look(e, m.id)}
            >
              <span className="duplicate-set__frame">
                <img src={mediaUrl("thumb", m.id, { s: 256 })} alt="" loading="lazy" draggable={false} />
                {kept && <span className="duplicate-set__keep">Keep</span>}
              </span>
              <span className="duplicate-set__meta">
                {m.width}×{m.height} · {m.format} · {formatBytes(Number(m.bytes))}
                {m.rating > 0 && (
                  <span className="duplicate-set__mark" aria-label={`${m.rating} stars`}>
                    <IconStarFilled size={11} /> {m.rating}
                  </span>
                )}
                {m.favorite && <IconHeartFilled size={11} className="duplicate-set__mark" aria-label="Favorite" />}
              </span>
            </button>
          );
        })}
      </div>
      <div className="duplicate-set__actions">
        <Button size="xs" onClick={onRemove}>
          Remove {others}
        </Button>
        <Button size="xs" variant="subtle" onClick={onDismiss}>
          Not duplicates
        </Button>
      </div>
    </section>
  );
}
