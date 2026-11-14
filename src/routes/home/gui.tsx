import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { IconFolderOpen, IconHome, IconStarFilled } from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { toast } from "@/components/toast";
import { deviceIcon } from "@/components/sidebar";
import { useDevices } from "@/hooks/use-devices";
import { useImport } from "@/hooks/use-import";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { DeviceSummary } from "@/types/generated/DeviceSummary";
import type { LibraryStats } from "@/types/generated/LibraryStats";
import { formatRelative } from "@/utils/date";
import { formatBytes } from "@/utils/format";
import { mediaUrl } from "@/utils/media";
import { AttentionStrip } from "./attention";
import { ContinueRow } from "./continue";
import { LibraryGlance } from "./glance";
import { MemoriesBlock } from "./hero";
import "./styles.scss";

function FavoritesStrip() {
  const [, setParams] = useSearchParams();
  const ids = useQuery({
    queryKey: queryKeys.favorites,
    queryFn: () => invoke<number[]>("get_favorite_ids", { limit: 12 }),
  });
  if (!ids.data?.length) return <p className="home-route__muted">Double-click a photo to star it.</p>;
  return (
    <div className="home-route__strip">
      {ids.data.map((id) => (
        <button
          key={id}
          type="button"
          className="home-route__thumb"
          aria-label={`Open favorite ${id}`}
          onClick={() =>
            setParams((p) => {
              p.set("asset", String(id));
              return p;
            })
          }
        >
          <img src={mediaUrl("thumb", id, { s: 256 })} alt="" loading="lazy" decoding="async" />
        </button>
      ))}
    </div>
  );
}

/** Home lists this many devices; the rest wait behind *Show all*. With the sidebar's Devices section hidden (Round 20), this is where devices live. */
export const HOME_DEVICES = 8;

/** Whether Home's device list is expanded, for the rest of the session. */
let devicesExpanded = false;

function DeviceTile({ d }: { d: DeviceSummary }) {
  const Icon = deviceIcon(d.kind, d.group);
  return (
    <Link to={`/library/${d.id}`} className="home-route__device rise-in">
      <Icon size={20} />
      <span className="home-route__device-name">{d.displayName}</span>
      <span className="home-route__device-sub">
        {d.itemCount.toLocaleString()} items{d.lastImportAt ? ` · ${formatRelative(d.lastImportAt)}` : ""}
      </span>
    </Link>
  );
}

function DevicesSection() {
  const { visible } = useDevices();
  const [expanded, setExpanded] = useState(devicesExpanded);
  if (visible.length === 0) return null;
  // Most recently imported first, then the fullest.
  const sorted = [...visible].sort((a, b) => (b.lastImportAt ?? 0) - (a.lastImportAt ?? 0) || b.itemCount - a.itemCount);
  const first = sorted.slice(0, HOME_DEVICES);
  const rest = sorted.slice(HOME_DEVICES);
  const toggle = () => {
    devicesExpanded = !expanded;
    setExpanded(devicesExpanded);
  };
  return (
    <section className="home-route__section" aria-label="Devices">
      <h2>Devices</h2>
      <div className="home-route__devices">
        {first.map((d) => (
          <DeviceTile key={d.id} d={d} />
        ))}
      </div>
      <AnimatePresence initial={false}>
        {expanded && rest.length > 0 && (
          <motion.div
            key="rest"
            className="home-route__devices-more"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
          >
            <div className="home-route__devices">
              {rest.map((d) => (
                <DeviceTile key={d.id} d={d} />
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      {rest.length > 0 && (
        <Button size="xs" variant="subtle" className="home-route__devices-toggle" onClick={toggle}>
          {expanded ? "Show fewer" : `Show all ${sorted.length.toLocaleString()}`}
        </Button>
      )}
    </section>
  );
}

/**
 * Home (Round 20 §10): what needs attention, a hero memory, where you left off, your library at
 * a glance, favorites, devices, the stats line. A section with nothing to show is not drawn.
 */
export function HomeRoute() {
  const navigate = useNavigate();
  const stats = useQuery({ queryKey: queryKeys.stats, queryFn: () => invoke<LibraryStats>("get_library_stats") });
  const { startFolderImport } = useImport();

  const pick = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose a folder to import" });
    if (typeof dir !== "string") return;
    try {
      await startFolderImport({ path: dir });
      navigate("/import");
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    }
  };

  const empty = stats.data !== undefined && stats.data.items === 0;

  return (
    <div className="home-route">
      <PageHeader title="Home" />
      {empty ? (
        <EmptyState
          icon={<IconHome />}
          title="Your library is empty"
          description="Import a folder, a card or a phone to get started."
          action={
            <Button leftSection={<IconFolderOpen size={16} />} onClick={pick}>
              Choose a folder…
            </Button>
          }
        />
      ) : (
        <div className="home-route__content">
          <AttentionStrip />
          <MemoriesBlock />
          <ContinueRow />
          <LibraryGlance />
          <section className="home-route__section" aria-label="Favorites">
            <header className="home-route__batch-head">
              <h2>
                <IconStarFilled size={16} className="home-route__star" /> Favorites
              </h2>
              <Button size="xs" variant="subtle" component={Link} to="/library?favorites=1">
                See all
              </Button>
            </header>
            <FavoritesStrip />
          </section>
          <DevicesSection />
          {stats.data && (
            <p className="home-route__stats" aria-label="Library stats">
              {stats.data.items.toLocaleString()} items · {stats.data.videos.toLocaleString()} videos · {formatBytes(stats.data.bytes)}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
