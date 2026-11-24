import type { DevelopSession } from "@/hooks/use-develop-session";
import { History, Snapshots } from "./history";
import { Presets } from "./presets";
import "./styles.scss";

/** Develop's left column (spec §1): presets, history and snapshots, stacked so more can join. */
export function DevelopSidebar({ session, assetId }: { session: DevelopSession; assetId: number }) {
  return (
    <div className="develop-sidebar" data-part="panel">
      <Presets session={session} />
      <Snapshots session={session} assetId={assetId} />
      <History session={session} assetId={assetId} />
    </div>
  );
}
