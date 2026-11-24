import { Button } from "@mantine/core";
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { PERMISSION_LABELS, trustLine } from "@/components/plugin-install";
import { paletteContributions, runContribution } from "@/lib/plugin-slots";
import { pluginLog } from "@/lib/plugins";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginKind } from "@/types/generated/PluginKind";

const ADDS: Record<PluginKind, string> = {
  presets: "Adds Develop presets",
  apps: "Adds apps to Open with",
  prompts: "Adds prompts for AI assistants",
  importer: "Adds an importer",
  tools: "Adds tools",
};

/** What a live plugin adds to the app's toolbars, menus and palette, runnable from its page too. */
function ContributionButtons({ p }: { p: PluginInfo }) {
  const navigate = useNavigate();
  const list = p.contributions?.length ? paletteContributions([p]) : [];
  if (list.length === 0) return null;
  return (
    <div className="plugin-page__label">
      {list.map((c, i) => (
        <span key={c.id}>
          {i > 0 && " "}
          <Button size="xs" variant={i === 0 ? "filled" : "default"} onClick={() => void runContribution(c, { navigate })}>
            {c.label}
          </Button>
        </span>
      ))}
    </div>
  );
}

/** What a plugin is, adds, may do and is licensed under (the old Details tab). */
export function PluginDetails({ p }: { p: PluginInfo }) {
  return (
    <div className="plugin-page__pane">
      <div className="plugin-page__muted">
        {p.author} · {trustLine(p.trust)}
      </div>
      {p.description && <p className="plugin-page__text">{p.description}</p>}
      <ContributionButtons p={p} />
      <div className="plugin-page__label">Adds</div>
      <ul className="plugin-page__list">
        {p.kinds.map((k) => (
          <li key={k}>{ADDS[k]}</li>
        ))}
      </ul>
      <div className="plugin-page__label">Permissions</div>
      {p.permissions.length === 0 ? (
        <p className="plugin-page__text">No permissions.</p>
      ) : (
        <ul className="plugin-page__list">
          {p.permissions.map((perm) => (
            <li key={perm}>{PERMISSION_LABELS[perm]}</li>
          ))}
        </ul>
      )}
      <div className="plugin-page__label">Licence</div>
      <p className="plugin-page__text">{p.license}</p>
      {p.assets.length > 0 && (
        <>
          <div className="plugin-page__label">Assets</div>
          <ul className="plugin-page__list">
            {p.assets.map((a) => (
              <li key={a.path}>
                {a.path} <span className="plugin-page__muted">· {a.license}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/** A plugin's log, for Developer mode. */
export function PluginLog({ id }: { id: string }) {
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
  if (lines.length === 0) return <p className="plugin-page__text">Nothing logged yet.</p>;
  return (
    <div ref={box} className="plugin-page__log" role="log" aria-label="Plugin log" tabIndex={0}>
      {lines.map((l, i) => (
        <div key={i} className="plugin-page__log-line">
          {l}
        </div>
      ))}
    </div>
  );
}
