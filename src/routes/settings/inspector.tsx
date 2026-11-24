import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { ThemePreview } from "@/components/theme-preview";
import { useLibraries } from "@/hooks/use-libraries";
import { useUpdateStatus } from "@/hooks/use-updates";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { LibraryStats } from "@/types/generated/LibraryStats";
import type { Settings } from "@/types/generated/Settings";
import { themeName } from "@/types/theme";
import { resolveTheme, useInstalledThemes } from "@/lib/plugin-theme";
import { formatStorage } from "@/utils/format";
import { usePeoplePick } from "./people";
import { usePointed } from "./search";
import { mlStatusQuery, type Section } from "./sections";
import { storageQuery } from "./storage";
import { updateLine } from "./updates";

/**
 * The right-hand panel of sample B (2026-11): a card with what the section is doing — the
 * theme in miniature, how far the models have got, what the library holds, this copy's
 * version, the people removed — and, for row sections, the whole explanation of the row the
 * pointer or focus is on (rows show one line of it).
 */
export function Inspector({ section, status, settings }: { section: Section; status?: string | null; settings: Settings }) {
  const Card = CARDS[section.id];
  return (
    <aside className="settings-inspector" aria-label={`About ${section.label}`}>
      {Card ? <Card settings={settings} /> : <StatusCard label={section.label} status={status} />}
      {section.layout !== "tiles" && <About />}
    </aside>
  );
}

const CARDS: Record<string, (p: { settings: Settings }) => ReactNode> = {
  appearance: AppearanceCard,
  smart: ModelsCard,
  library: LibraryCard,
  updates: UpdatesCard,
  people: PeopleCard,
};

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="settings-inspector__card" data-part="card">
      <span className="settings-inspector__key">{title}</span>
      {children}
    </div>
  );
}

/** Label–value pairs; a value of `null` leaves its pair out. */
function Facts({ items }: { items: [string, ReactNode][] }) {
  return (
    <dl className="settings-inspector__facts">
      {items
        .filter(([, v]) => v != null)
        .map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
    </dl>
  );
}

function Meter({ label, value }: { label: string; value: number }) {
  const pct = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <div className="settings-inspector__meter" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
      <i style={{ width: `${pct}%` }} />
    </div>
  );
}

function StatusCard({ label, status }: { label: string; status?: string | null }) {
  return (
    <Card title={label}>
      <span className="settings-inspector__big">{status || "Ready"}</span>
    </Card>
  );
}

const osDark = () => typeof window !== "undefined" && !!window.matchMedia?.("(prefers-color-scheme: dark)").matches;

function AppearanceCard({ settings }: { settings: Settings }) {
  const a = settings.appearance;
  const pluginThemes = useInstalledThemes() ?? [];
  const preview = resolveTheme(a.theme, pluginThemes);
  const effective = a.scheme === "auto" ? (osDark() ? "dark" : "light") : a.scheme;
  const mode = a.scheme === "auto" ? `Automatic (${effective === "dark" ? "dark" : "light"} now)` : a.scheme === "dark" ? "Dark" : "Light";
  return (
    <Card title="Now">
      <div className="settings-inspector__preview">
        <ThemePreview theme={preview.base} plugin={preview.plugin} scheme={effective} />
      </div>
      <Facts
        items={[
          ["Theme", themeName(a.theme, pluginThemes)],
          ["Mode", mode],
          ["Motion", a.reduceMotion ? "Reduced" : "Full"],
        ]}
      />
    </Card>
  );
}

function ModelsCard({ settings }: { settings: Settings }) {
  const ml = useQuery(mlStatusQuery).data;
  if (!(settings.smart.enabled ?? true)) {
    return (
      <Card title="Analysed">
        <span className="settings-inspector__big">Off</span>
        <span className="settings-inspector__text">Memories, music and slideshows use Lumen's own rules.</span>
      </Card>
    );
  }
  const pct = ml && ml.photos > 0 ? (ml.embedded / ml.photos) * 100 : 0;
  const missing = ml?.models.filter((m) => !m.ready).reduce((s, m) => s + m.bytes, 0) ?? 0;
  return (
    <Card title="Analysed">
      <span className="settings-inspector__big">{ml ? `${Math.floor(pct)}%` : "…"}</span>
      <Meter label="Analysed" value={pct} />
      <Facts
        items={[
          ["Photos", ml ? `${ml.embedded.toLocaleString()} / ${ml.photos.toLocaleString()}` : null],
          ["People", ml ? ml.people.toLocaleString() : null],
          ["Faces", ml ? ml.faces.toLocaleString() : null],
          ["To download", missing > 0 ? formatStorage(missing) : null],
        ]}
      />
    </Card>
  );
}

function LibraryCard() {
  const stats = useQuery({ queryKey: queryKeys.stats, queryFn: () => invoke<LibraryStats>("get_library_stats") }).data;
  const { current } = useLibraries();
  const disk = useQuery(storageQuery).data?.disks.find((d) => d.holdsLibrary);
  const used = disk && disk.total > 0 ? ((disk.total - disk.available) / disk.total) * 100 : null;
  return (
    <Card title={current?.name ?? "Library"}>
      <span className="settings-inspector__big">{stats ? stats.items.toLocaleString() : "…"}</span>
      <Facts
        items={[
          ["Photos", stats ? (stats.items - stats.videos).toLocaleString() : null],
          ["Videos", stats ? stats.videos.toLocaleString() : null],
          ["Size", stats ? formatStorage(stats.bytes) : null],
          [disk ? `Free on ${disk.label || disk.mount.replace(/[\\/]$/, "")}` : "Free", disk ? formatStorage(disk.available) : null],
        ]}
      />
      {used != null && <Meter label="Disk used" value={used} />}
    </Card>
  );
}

function UpdatesCard() {
  const { status, check } = useUpdateStatus();
  const state = status?.state;
  const canCheck = state && state.state !== "off" && state.state !== "notSetUp" && state.state !== "downloading" && state.state !== "ready";
  return (
    <Card title="This copy">
      <span className="settings-inspector__big">{status?.version ?? "…"}</span>
      {state && <span className="settings-inspector__text">{updateLine(state)}</span>}
      {canCheck && (
        <Button size="xs" loading={state.state === "checking"} onClick={() => void check()}>
          Check now
        </Button>
      )}
    </Card>
  );
}

function PeopleCard() {
  const { removed, live, restoreMany } = usePeoplePick();
  return (
    <Card title="Removed">
      <span className="settings-inspector__big">{removed.length.toLocaleString()}</span>
      <span className="settings-inspector__text">Restored people go back to People with their photos.</span>
      <Button size="xs" disabled={live.length === 0} onClick={() => restoreMany(live)}>
        {live.length > 0 ? `Restore ${live.length} selected` : "Restore selected"}
      </Button>
    </Card>
  );
}

/** The pointed or focused row's whole label and hint. */
function About() {
  const pointed = usePointed();
  return (
    <div className="settings-inspector__card">
      <span className="settings-inspector__key">About this setting</span>
      <div key={pointed ? String(pointed.label) + String(pointed.hint) : "none"} className="settings-inspector__about" aria-live="polite">
        {pointed ? (
          <>
            <b>{pointed.label}</b>
            {pointed.hint}
          </>
        ) : (
          <>
            <b>Point at a setting</b>
            Its whole explanation shows here.
          </>
        )}
      </div>
    </div>
  );
}
