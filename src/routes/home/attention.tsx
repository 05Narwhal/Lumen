import { IconArrowDown, IconCopy, IconDeviceMobile, IconFileAlert, IconUserQuestion } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import type { ComponentType, ReactNode } from "react";
import { Link } from "react-router";
import { useVolumes } from "@/hooks/use-devices";
import { useHomeSummary } from "@/hooks/use-home";
import { useIphones } from "@/hooks/use-iphones";
import { connectedPath } from "@/hooks/use-connected-menu";
import { useUpdateStatus } from "@/hooks/use-updates";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";

interface Pill {
  key: string;
  to: string;
  icon: ComponentType<{ size?: number }>;
  text: ReactNode;
}

const sets = (n: number) => (n === 1 ? "1 duplicate set" : `${n.toLocaleString()} duplicate sets`);

/**
 * What needs attention (Round 20 §10.1): one line of pills, drawn only when something applies.
 * A card or camera with new items, a connected iPhone, duplicates, people to name, files Lumen
 * can't open, an update that is ready.
 */
export function AttentionStrip() {
  const { volumes } = useVolumes();
  const { devices: iphones } = useIphones();
  const { summary } = useHomeSummary();
  const { status } = useUpdateStatus();
  const duplicates = useQuery({ queryKey: queryKeys.duplicateCount, queryFn: () => invoke<[number, number]>("count_duplicate_sets") });

  const pills: Pill[] = [];
  for (const v of volumes) {
    if (v.newItems > 0) {
      pills.push({
        key: `volume:${v.mount}`,
        to: connectedPath({ kind: "volume", id: v.mount }),
        icon: IconArrowDown,
        text: `Import ${v.newItems.toLocaleString()} new · ${v.label || v.mount}`,
      });
    }
  }
  for (const d of iphones) {
    pills.push({ key: `iphone:${d.udid}`, to: connectedPath({ kind: "iphone", id: d.udid }), icon: IconDeviceMobile, text: `${d.name} is connected` });
  }
  const duplicateSets = duplicates.data?.[0] ?? 0;
  if (duplicateSets > 0) pills.push({ key: "duplicates", to: "/duplicates", icon: IconCopy, text: sets(duplicateSets) });
  if (summary && summary.peopleToReview > 0) {
    pills.push({ key: "people", to: "/people", icon: IconUserQuestion, text: `${summary.peopleToReview.toLocaleString()} to name` });
  }
  if (summary && summary.unreadable > 0) {
    const n = summary.unreadable;
    pills.push({
      key: "unreadable",
      to: "/import?mode=repair",
      icon: IconFileAlert,
      text: n === 1 ? "1 file Lumen can't open" : `${n.toLocaleString()} files Lumen can't open`,
    });
  }
  if (status?.state.state === "ready") {
    pills.push({ key: "update", to: "/settings?section=updates", icon: IconArrowDown, text: `Update ${status.state.version} is ready` });
  }

  if (pills.length === 0) return null;
  return (
    <nav className="home-route__attention rise-in" aria-label="Needs attention">
      {pills.map(({ key, to, icon: Icon, text }) => (
        <Link key={key} to={to} className="home-route__pill">
          <Icon size={16} />
          {text}
        </Link>
      ))}
    </nav>
  );
}
