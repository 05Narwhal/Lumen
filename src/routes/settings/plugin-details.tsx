import { Modal, Select, Switch, Tabs, TextInput } from "@mantine/core";
import { useEffect, useRef, useState } from "react";
import { PERMISSION_LABELS, trustLine } from "@/components/plugin-install";
import { toast } from "@/components/toast";
import { usePlugins } from "@/hooks/use-plugins";
import { isAppError } from "@/lib/ipc";
import { pluginLog, setPluginSetting } from "@/lib/plugins";
import { createStore } from "@/lib/store";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginKind } from "@/types/generated/PluginKind";
import type { PluginSetting } from "@/types/generated/PluginSetting";

export type PluginDetailsTab = "details" | "settings" | "log";

/** Which plugin's details panel is open, and on which tab (`null` = closed). */
export const pluginDetailsStore = createStore<{ id: string; tab: PluginDetailsTab } | null>(null);

export const openPluginDetails = (id: string, tab: PluginDetailsTab) => pluginDetailsStore.set({ id, tab });
export const closePluginDetails = () => pluginDetailsStore.set(null);

const ADDS: Record<PluginKind, string> = {
  theme: "Adds a theme",
  presets: "Adds Develop presets",
  apps: "Adds apps to Open with",
  prompts: "Adds prompts for AI assistants",
  importer: "Adds an importer",
  tools: "Adds tools",
};

function Details({ p }: { p: PluginInfo }) {
  return (
    <div className="plugin-details__pane">
      <div className="plugin-details__title">
        {p.name} <span className="plugin-details__muted">{p.version}</span>
      </div>
      <div className="plugin-details__muted">
        {p.author} · {trustLine(p.trust)}
      </div>
      {p.description && <p className="plugin-details__text">{p.description}</p>}
      <div className="plugin-details__label">Adds</div>
      <ul className="plugin-details__list">
        {p.kinds.map((k) => (
          <li key={k}>{ADDS[k]}</li>
        ))}
      </ul>
      <div className="plugin-details__label">Permissions</div>
      {p.permissions.length === 0 ? (
        <p className="plugin-details__text">No permissions.</p>
      ) : (
        <ul className="plugin-details__list">
          {p.permissions.map((perm) => (
            <li key={perm}>{PERMISSION_LABELS[perm]}</li>
          ))}
        </ul>
      )}
      <div className="plugin-details__label">Licence</div>
      <p className="plugin-details__text">{p.license}</p>
      {p.assets.length > 0 && (
        <>
          <div className="plugin-details__label">Assets</div>
          <ul className="plugin-details__list">
            {p.assets.map((a) => (
              <li key={a.path}>
                {a.path} <span className="plugin-details__muted">· {a.license}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

const save = (id: string, key: string, value: unknown) =>
  setPluginSetting(id, key, value).catch((e) => toast({ message: isAppError(e) ? e.message : "Could not save that setting." }));

/** Text kept while typing; saved on blur or Enter, and re-read when the stored value changes under it. */
function useDraft(stored: string, same: (raw: string) => boolean) {
  const [raw, setRaw] = useState(stored);
  const focused = useRef(false);
  const saved = useRef(stored);
  useEffect(() => {
    saved.current = stored;
    if (!focused.current || !same(raw)) setRaw(stored);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stored]);
  return { raw, setRaw, focused, saved };
}

function NumberField({ id, s }: { id: string; s: PluginSetting }) {
  const stored = String(s.value ?? "");
  const { raw, setRaw, focused, saved } = useDraft(stored, (r) => Number(r) === Number(stored));
  const valid = raw.trim() !== "" && Number.isFinite(Number(raw));
  const commit = () => {
    if (!valid || String(Number(raw)) === saved.current) return;
    saved.current = String(Number(raw));
    void save(id, s.key, Number(raw));
  };
  return (
    <TextInput
      label={s.label}
      inputMode="decimal"
      value={raw}
      error={valid ? undefined : "Enter a number"}
      onFocus={() => (focused.current = true)}
      onChange={(e) => setRaw(e.currentTarget.value)}
      onBlur={() => {
        focused.current = false;
        commit();
      }}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}

function TextField({ id, s }: { id: string; s: PluginSetting }) {
  const stored = String(s.value ?? "");
  const { raw, setRaw, focused, saved } = useDraft(stored, (r) => r === stored);
  const commit = () => {
    if (raw === saved.current) return;
    saved.current = raw;
    void save(id, s.key, raw);
  };
  return (
    <TextInput
      label={s.label}
      value={raw}
      onFocus={() => (focused.current = true)}
      onChange={(e) => setRaw(e.currentTarget.value)}
      onBlur={() => {
        focused.current = false;
        commit();
      }}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}

function SettingControl({ id, s }: { id: string; s: PluginSetting }) {
  const [on, setOn] = useState(s.value === true);
  const [choice, setChoice] = useState<string | null>(typeof s.value === "string" ? s.value : null);
  useEffect(() => setOn(s.value === true), [s.value]);
  useEffect(() => setChoice(typeof s.value === "string" ? s.value : null), [s.value]);
  if (s.kind === "bool")
    return (
      <Switch
        label={s.label}
        checked={on}
        onChange={(e) => {
          setOn(e.currentTarget.checked);
          void save(id, s.key, e.currentTarget.checked);
        }}
      />
    );
  if (s.kind === "number") return <NumberField id={id} s={s} />;
  if (s.kind === "choice")
    return (
      <Select
        label={s.label}
        data={s.choices}
        value={choice}
        allowDeselect={false}
        onChange={(v) => {
          if (v === null) return;
          setChoice(v);
          void save(id, s.key, v);
        }}
      />
    );
  return <TextField id={id} s={s} />;
}

function SettingsForm({ p }: { p: PluginInfo }) {
  if (p.settings.length === 0) return <p className="plugin-details__text">This plugin has no settings.</p>;
  return (
    <div className="plugin-details__pane plugin-details__form">
      {p.settings.map((s) => (
        <SettingControl key={`${p.id}:${s.key}`} id={p.id} s={s} />
      ))}
    </div>
  );
}

function Log({ id }: { id: string }) {
  const box = useRef<HTMLDivElement>(null);
  const [lines, setLines] = useState<string[] | null>(null);
  useEffect(() => {
    let live = true;
    pluginLog(id).then(
      (l) => live && setLines(l),
      () => live && setLines([]),
    );
    return () => {
      live = false;
    };
  }, [id]);
  useEffect(() => {
    if (box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines]);
  if (lines === null) return null;
  if (lines.length === 0) return <p className="plugin-details__text">Nothing logged yet.</p>;
  return (
    <div ref={box} className="plugin-details__log" role="log" aria-label="Plugin log" tabIndex={0}>
      {lines.map((l, i) => (
        <div key={i} className="plugin-details__log-line">
          {l}
        </div>
      ))}
    </div>
  );
}

/** The panel behind a plugin row's Details / Settings / Log (Round 23 §3 and §5). */
export function PluginDetailsHost() {
  const open = pluginDetailsStore.use();
  const { plugins, developerMode } = usePlugins();
  const p = open ? plugins.find((x) => x.id === open.id) : undefined;
  const tab = open ? (open.tab === "log" && !developerMode ? "details" : open.tab) : "details";
  return (
    <Modal
      opened={!!open && !!p}
      onClose={closePluginDetails}
      title={p?.name}
      centered
      size="md"
      classNames={{ content: "plugin-details", header: "plugin-details__header", body: "plugin-details__body" }}
    >
      {open && p && (
        <Tabs keepMounted={false} value={tab} onChange={(t) => t && openPluginDetails(open.id, t as PluginDetailsTab)}>
          <Tabs.List>
            <Tabs.Tab value="details">Details</Tabs.Tab>
            {p.settings.length > 0 && <Tabs.Tab value="settings">Settings</Tabs.Tab>}
            {developerMode && <Tabs.Tab value="log">Log</Tabs.Tab>}
          </Tabs.List>
          <Tabs.Panel value="details">
            <Details p={p} />
          </Tabs.Panel>
          <Tabs.Panel value="settings">
            <SettingsForm p={p} />
          </Tabs.Panel>
          {developerMode && (
            <Tabs.Panel value="log">
              <Log id={p.id} />
            </Tabs.Panel>
          )}
        </Tabs>
      )}
    </Modal>
  );
}
