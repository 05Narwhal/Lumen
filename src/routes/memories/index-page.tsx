import { Switch } from "@mantine/core";
import { IconClock } from "@tabler/icons-react";
import { useState } from "react";
import { useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { MemoryCard } from "@/components/memory-card";
import { PageHeader } from "@/components/page-header";
import { Segmented } from "@/components/segmented";
import { MEMORY_KIND, useMemories } from "@/hooks/use-memories";
import { SlideshowsActions, SlideshowsBody, useSlideshowStart } from "@/routes/slideshows/index-page";

const GROUPS = [
  { kind: MEMORY_KIND.onThisDay, title: "On this day" },
  { kind: MEMORY_KIND.trip, title: "Trips" },
  { kind: MEMORY_KIND.event, title: "Events" },
];

type Tab = "memories" | "slideshows";
const TABS = [
  { label: "Memories", value: "memories" as const },
  { label: "Slideshows", value: "slideshows" as const },
];

/**
 * Every memory as a card, grouped by kind (UX spec §6.5); hidden ones on request. Slideshows
 * are a tab here (Round 20): `?tab=slideshows`, absent meaning memories. The toolbar on the
 * right is the tab's own.
 */
export function MemoriesIndex() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = params.get("tab") === "slideshows" ? "slideshows" : "memories";
  const [showHidden, setShowHidden] = useState(false);
  const { memories, isLoading } = useMemories({ includeHidden: showHidden });
  const start = useSlideshowStart();
  const toggle = <Switch size="sm" label="Show hidden" checked={showHidden} onChange={(e) => setShowHidden(e.currentTarget.checked)} />;
  const tabs = <Segmented label="Show" size="sm" data={TABS} value={tab} onChange={(v) => setParams(v === "slideshows" ? { tab: v } : {}, { replace: true })} />;
  return (
    <div className="memories-route">
      <PageHeader
        title="Memories"
        search={false}
        count={tabs}
        toolbar={tab === "slideshows" ? <SlideshowsActions start={start} /> : memories.length > 0 || showHidden ? toggle : undefined}
      />
      {tab === "slideshows" ? (
        <SlideshowsBody start={start} />
      ) : memories.length === 0 ? (
        !isLoading && <EmptyState icon={<IconClock />} title="No memories yet" description="Memories appear as your library grows." />
      ) : (
        <div className="memories-route__body">
          {GROUPS.map(({ kind, title }) => {
            const group = memories.filter((m) => m.kind === kind);
            if (group.length === 0) return null;
            return (
              <section key={kind} className="memories-route__group" aria-label={title}>
                <h2>{title}</h2>
                <div className="memories-route__grid">
                  {group.map((m) => (
                    <MemoryCard key={m.id} memory={m} />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      )}
      {start.picker}
    </div>
  );
}
