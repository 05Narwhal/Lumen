import { IconPlayerPlayFilled } from "@tabler/icons-react";
import { useState } from "react";
import { Link } from "react-router";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { mediaUrl } from "@/utils/media";
import { useMemoryActions } from "./use-memory-actions";
import "./styles.scss";

/**
 * A memory as a 3:2 card: its cover, title and dates. Links to the memory; the round button
 * plays its film (Round 4F). Right-click for Play · Open · Make slideshow · Export photos… ·
 * Hide memory (Round 4G).
 */
export function MemoryCard({ memory }: { memory: MemorySummary }) {
  const [loaded, setLoaded] = useState(false);
  const { play, onContextMenu } = useMemoryActions(memory);
  return (
    <div className="memory-card" data-hidden={memory.hidden} onContextMenu={onContextMenu}>
      <Link to={`/memories/${memory.id}`} className="memory-card__link">
        {memory.coverAssetId != null && (
          <img className="memory-card__img" src={mediaUrl("thumb", memory.coverAssetId, { s: 1024 })} alt="" loading="lazy" data-loaded={loaded} onLoad={() => setLoaded(true)} />
        )}
        <span className="memory-card__text">
          <strong className="memory-card__title">{memory.title}</strong>
          {memory.subtitle && <span className="memory-card__subtitle">{memory.subtitle}</span>}
        </span>
      </Link>
      <button type="button" className="memory-card__play" aria-label={`Play ${memory.title}`} title="Play" onClick={play}>
        <IconPlayerPlayFilled size={16} />
      </button>
    </div>
  );
}
