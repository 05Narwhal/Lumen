import { Button } from "@mantine/core";
import { IconPlayerPlayFilled } from "@tabler/icons-react";
import { useMemo } from "react";
import { Link } from "react-router";
import { MemoryCard, useMemoryActions } from "@/components/memory-card";
import { MEMORY_KIND, useMemories } from "@/hooks/use-memories";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { mediaUrl } from "@/utils/media";
import { pickHero } from "./pick-hero";

/** How many more memories sit beside the hero (two on a wide page, a scrolling row on a narrow one). */
const BESIDE = 8;

/** *On this day · 3 years ago*, *Trip*, *Event*. Memories carry wall-clock seconds, so the year is read as UTC. */
export function kicker(memory: MemorySummary, today: Date): string {
  if (memory.kind === MEMORY_KIND.onThisDay) {
    const years = today.getFullYear() - new Date(memory.startAt * 1000).getUTCFullYear();
    return years >= 1 ? `On this day · ${years === 1 ? "1 year" : `${years} years`} ago` : "On this day";
  }
  return memory.kind === MEMORY_KIND.trip ? "Trip" : "Event";
}

/** An *On this day* memory's subtitle is "2024 · 2 years ago"; the kicker already says how long ago. */
function subtitle(memory: MemorySummary): string | null {
  return memory.kind === MEMORY_KIND.onThisDay ? (memory.subtitle?.split(" · ")[0] ?? null) : memory.subtitle;
}

function Hero({ memory, today }: { memory: MemorySummary; today: Date }) {
  const { play, onContextMenu } = useMemoryActions(memory);
  const photos = memory.itemCount === 1 ? "1 photo" : `${memory.itemCount.toLocaleString()} photos`;
  return (
    <div className="home-hero rise-in" aria-label="Featured memory" role="group" onContextMenu={onContextMenu}>
      {memory.coverAssetId != null && <img className="home-hero__img" src={mediaUrl("preview", memory.coverAssetId)} alt="" decoding="async" />}
      <Link to={`/memories/${memory.id}`} className="home-hero__link" aria-label={`Open ${memory.title}`} />
      <div className="home-hero__text">
        <span className="home-hero__kicker">{kicker(memory, today)}</span>
        <h2 className="home-hero__title">{memory.title}</h2>
        <span className="home-hero__sub">{[subtitle(memory), photos].filter(Boolean).join(" · ")}</span>
        <div className="home-hero__actions">
          <Button size="xs" leftSection={<IconPlayerPlayFilled size={14} />} onClick={play}>
            Play
          </Button>
          <Button size="xs" variant="default" component={Link} to={`/memories/${memory.id}`}>
            Open
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * Home's hero and the memories beside it (Round 20 §10.2). The block is not drawn without
 * memories; while they load its height is held, with no spinner.
 */
export function MemoriesBlock() {
  const { memories, isLoading } = useMemories();
  const today = useMemo(() => new Date(), []);
  const hero = useMemo(() => pickHero(memories, today), [memories, today]);
  if (isLoading) return <div className="home-hero home-hero--reserved" aria-hidden />;
  if (!hero) return null;
  const beside = memories.filter((m) => !m.hidden && m.id !== hero.id).slice(0, BESIDE);
  return (
    <section className="home-route__memories" aria-label="Memories">
      <Hero memory={hero} today={today} />
      <div className="home-route__beside">
        <div className="home-route__beside-list">
          {beside.map((m) => (
            <div className="home-route__beside-item" key={m.id}>
              <MemoryCard memory={m} />
            </div>
          ))}
        </div>
        <Button size="xs" variant="subtle" component={Link} to="/memories" className="home-route__beside-all">
          See all
        </Button>
      </div>
    </section>
  );
}
