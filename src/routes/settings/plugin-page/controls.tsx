import { Button, SegmentedControl, Select, Slider, Switch, TextInput } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import { toast } from "@/components/toast";
import { isAppError } from "@/lib/ipc";
import { pluginUiAction, pluginUiValue, setPluginSetting } from "@/lib/plugins";
import type { PluginControl } from "@/types/generated/PluginControl";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { SettingRow } from "../row";

const save = (id: string, key: string, value: unknown) =>
  setPluginSetting(id, key, value).catch((e) => toast({ message: isAppError(e) ? e.message : "Could not save that setting." }));

const openLink = (url: string) => void openUrl(url).catch(() => window.open(url, "_blank", "noopener"));

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

function NumberField({ id, c, disabled }: { id: string; c: PluginControl; disabled?: boolean }) {
  const stored = String(c.value ?? "");
  const { raw, setRaw, focused, saved } = useDraft(stored, (r) => Number(r) === Number(stored));
  const valid = raw.trim() !== "" && Number.isFinite(Number(raw));
  const commit = () => {
    if (!valid || String(Number(raw)) === saved.current) return;
    saved.current = String(Number(raw));
    void save(id, c.key as string, Number(raw));
  };
  return (
    <TextInput
      label={c.label}
      description={c.help ?? undefined}
      disabled={disabled}
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

function TextField({ id, c, disabled }: { id: string; c: PluginControl; disabled?: boolean }) {
  const stored = String(c.value ?? "");
  const { raw, setRaw, focused, saved } = useDraft(stored, (r) => r === stored);
  const commit = () => {
    if (raw === saved.current) return;
    saved.current = raw;
    void save(id, c.key as string, raw);
  };
  return (
    <TextInput
      label={c.label}
      description={c.help ?? undefined}
      disabled={disabled}
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

/** A `stat` or `list` value, asked of the plugin; empty while it is off, stopped, or the call fails. */
function useUiValue(p: PluginInfo, key: string, running: boolean) {
  const { data } = useQuery({
    queryKey: ["plugin-ui-value", p.id, key, p.version],
    queryFn: () => pluginUiValue(p.id, key).catch(() => ""),
    enabled: running,
  });
  return running ? (data ?? "") : "";
}

function Stat({ p, c, running }: { p: PluginInfo; c: PluginControl; running: boolean }) {
  const text = useUiValue(p, c.key as string, running);
  return (
    <SettingRow label={c.label} hint={c.help}>
      <span className="plugin-page__stat">{text || "—"}</span>
    </SettingRow>
  );
}

function parseList(raw: string): { text: string; detail?: string }[] {
  try {
    const v: unknown = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    return v.flatMap((x) => {
      if (typeof x === "string") return [{ text: x }];
      if (x && typeof x === "object" && typeof (x as { text?: unknown }).text === "string") {
        const d = (x as { detail?: unknown }).detail;
        return [{ text: (x as { text: string }).text, detail: typeof d === "string" ? d : undefined }];
      }
      return [];
    });
  } catch {
    return [];
  }
}

function ListValue({ p, c, running }: { p: PluginInfo; c: PluginControl; running: boolean }) {
  const items = parseList(useUiValue(p, c.key as string, running));
  return (
    <SettingRow label={c.label} hint={c.help} wide>
      {items.length === 0 ? (
        <span className="plugin-page__muted">Nothing to show.</span>
      ) : (
        <ul className="plugin-page__items">
          {items.map((it, i) => (
            <li key={i}>
              <span>{it.text}</span>
              {it.detail && <span className="plugin-page__muted">{it.detail}</span>}
            </li>
          ))}
        </ul>
      )}
    </SettingRow>
  );
}

function ActionButton({ p, c, disabled }: { p: PluginInfo; c: PluginControl; disabled: boolean }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [line, setLine] = useState("");
  const run = async () => {
    setBusy(true);
    try {
      setLine(await pluginUiAction(p.id, c.action as string));
    } catch (e) {
      setLine(isAppError(e) ? e.message : "That did not work.");
    }
    setBusy(false);
    void qc.invalidateQueries({ queryKey: ["plugin-ui-value", p.id] });
  };
  return (
    <SettingRow label={c.label} hint={c.help}>
      <Button size="xs" variant="default" loading={busy} disabled={disabled} onClick={() => void run()} aria-label={c.label}>
        {c.label}
      </Button>
      {line && (
        <span className="plugin-page__outcome" role="status">
          {line}
        </span>
      )}
    </SettingRow>
  );
}

/** One value control (toggle, select, segmented, slider, number, text); `disabled` while its app isn't found. */
function ValueControl({ id, c, disabled }: { id: string; c: PluginControl; disabled: boolean }) {
  const key = c.key as string;
  const [on, setOn] = useState(c.value === true);
  const [choice, setChoice] = useState<string | null>(typeof c.value === "string" ? c.value : null);
  const [level, setLevel] = useState(typeof c.value === "number" ? c.value : (c.min ?? 0));
  useEffect(() => setOn(c.value === true), [c.value]);
  useEffect(() => setChoice(typeof c.value === "string" ? c.value : null), [c.value]);
  useEffect(() => setLevel(typeof c.value === "number" ? c.value : (c.min ?? 0)), [c.value, c.min]);
  const row = (control: React.ReactNode, wide = false) => (
    <SettingRow label={c.label} hint={c.help} wide={wide}>
      {control}
    </SettingRow>
  );
  switch (c.kind) {
    case "toggle":
      return row(
        <Switch
          aria-label={c.label}
          disabled={disabled}
          checked={on}
          onChange={(e) => {
            setOn(e.currentTarget.checked);
            void save(id, key, e.currentTarget.checked);
          }}
        />,
      );
    case "select":
      return row(
        <Select
          aria-label={c.label}
          disabled={disabled}
          data={c.choices}
          value={choice}
          allowDeselect={false}
          onChange={(v) => {
            if (v === null) return;
            setChoice(v);
            void save(id, key, v);
          }}
        />,
      );
    case "segmented":
      return row(
        <SegmentedControl
          aria-label={c.label}
          disabled={disabled}
          data={c.choices}
          value={choice ?? c.choices[0]}
          onChange={(v) => {
            setChoice(v);
            void save(id, key, v);
          }}
        />,
      );
    case "slider":
      return row(
        <Slider
          className="plugin-page__slider"
          aria-label={c.label}
          disabled={disabled}
          min={c.min ?? 0}
          max={c.max ?? 100}
          step={c.step ?? 1}
          value={level}
          onChange={setLevel}
          onChangeEnd={(v) => void save(id, key, v)}
        />,
      );
    case "number":
      return (
        <SettingRow wide>
          <NumberField id={id} c={c} disabled={disabled} />
        </SettingRow>
      );
    default:
      return (
        <SettingRow wide>
          <TextField id={id} c={c} disabled={disabled} />
        </SettingRow>
      );
  }
}

/**
 * One declared control of a plugin's page. `requires` hides it until that control is on / non-empty;
 * `appFound` says whether the `app` it names was found (a control with an app is off until then).
 */
export function PluginControlView({ p, c, valueOf, appFound }: { p: PluginInfo; c: PluginControl; valueOf: (key: string) => unknown; appFound: (id: string) => boolean }) {
  if (c.requires) {
    const v = valueOf(c.requires);
    if (v === false || v === null || v === undefined || v === "" || v === 0) return null;
  }
  const blocked = p.state.type === "needsNewerLumen" || p.state.type === "problem" || p.state.type === "stopped";
  const running = p.enabled && !blocked;
  const noApp = c.app !== null && !appFound(c.app);
  switch (c.kind) {
    case "markdown":
      return (
        <SettingRow wide>
          <div className="plugin-page__markdown">
            <Markdown
              skipHtml
              components={{
                a: ({ href, children }) => (
                  <a
                    href={href}
                    onClick={(e) => {
                      e.preventDefault();
                      if (href) openLink(href);
                    }}
                  >
                    {children}
                  </a>
                ),
                img: () => null,
              }}
            >
              {c.body ?? ""}
            </Markdown>
          </div>
        </SettingRow>
      );
    case "button":
      return <ActionButton p={p} c={c} disabled={!running || noApp} />;
    case "stat":
      return <Stat p={p} c={c} running={running} />;
    case "list":
      return <ListValue p={p} c={c} running={running} />;
    default:
      return <ValueControl id={p.id} c={c} disabled={noApp} />;
  }
}
