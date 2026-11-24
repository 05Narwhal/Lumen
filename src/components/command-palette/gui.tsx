import { Modal } from "@mantine/core";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { useNavigate } from "react-router";
import { isMac } from "@/components/window-controls";
import { useSettings } from "@/hooks/use-settings";
import { appCommands, type AppCommand } from "@/lib/commands";
import { loadExternalApps } from "@/lib/external-apps";
import { keysFor } from "@/lib/keybinds";
import { runContribution, usePaletteContributions } from "@/lib/plugin-slots";
import { closePalette, paletteStore } from "./store";
import "./styles.scss";

/** What the list shows: a command, or an entry of a sub-list. */
interface Row {
  id: string;
  label: string;
  group: string;
  iconUrl?: string;
  run: AppCommand["run"];
}

const fmt = (s?: string) => s?.replace("Mod", isMac() ? "⌘" : "Ctrl");

export function CommandPalette() {
  const { open, sub } = paletteStore.use();
  const navigate = useNavigate();
  const { update } = useSettings();
  const qc = useQueryClient();
  const contributions = usePaletteContributions();
  const [q, setQ] = useState("");
  const [cursor, setCursor] = useState(0);

  // The apps arrive after the palette opens: the names *Edit in …* shows follow them.
  const [appsLoaded, setAppsLoaded] = useState(0);
  useEffect(() => {
    if (open && !sub) void loadExternalApps(qc).then(() => setAppsLoaded((n) => n + 1), () => {});
  }, [open, sub, qc]);

  const items = useMemo<Row[]>(() => {
    const n = q.trim().toLowerCase();
    if (sub) return sub.items.filter((i) => !n || i.label.toLowerCase().includes(n)).map((i) => ({ ...i, group: sub.title, run: () => i.run() }));
    const own = appCommands.filter((c) => (c.available?.(qc) ?? true) && (!n || c.label.toLowerCase().includes(n) || c.group.toLowerCase().includes(n)));
    // What plugins add (every contribution has an entry here), grouped under the plugin's name.
    const added: Row[] = contributions
      .filter((c) => !n || c.label.toLowerCase().includes(n) || c.pluginName.toLowerCase().includes(n))
      .map((c) => ({ id: c.id, label: c.label, group: c.pluginName, run: ({ navigate }) => void runContribution(c, { navigate }) }));
    const matches: Row[] = [...own.map((c) => ({ ...c, label: c.labelFor?.(qc) ?? c.label })), ...added];
    // Typed text that matches no command is almost certainly something to look for.
    if (n && matches.length === 0) {
      const text = q.trim();
      return [
        {
          id: "search-library",
          label: `Search: ${text}`,
          group: "Library" as const,
          run: ({ navigate }: { navigate: ReturnType<typeof useNavigate> }) =>
            navigate(`/search?q=${encodeURIComponent(text)}`),
        },
      ];
    }
    return matches;
    // `open`: an Experimental switch may have changed since the palette was last shown.
  }, [q, qc, open, sub, appsLoaded, contributions]);

  useEffect(() => {
    if (open) {
      setQ("");
      setCursor(0);
    }
  }, [open]);
  useEffect(() => setCursor(0), [q]);

  const run = (i: number) => {
    const c = items[i];
    if (!c) return;
    closePalette();
    c.run({ navigate, update, qc });
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, items.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(cursor);
    }
  };

  return (
    <Modal
      opened={open}
      onClose={closePalette}
      data-part="palette"
      withCloseButton={false}
      centered
      size="md"
      padding={0}
      classNames={{ content: "palette", body: "palette__body" }}
      transitionProps={{ duration: 120 }}
    >
      <input
        className="palette__input"
        aria-label="Command"
        placeholder={sub ? `${sub.title}…` : "Type a command…"}
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={onKey}
        autoFocus
      />
      <ul className="palette__list" role="listbox" aria-label="Commands">
        {items.map((c, i) => (
          <li
            key={c.id}
            role="option"
            aria-selected={i === cursor}
            className="palette__item"
            onMouseEnter={() => setCursor(i)}
            onClick={() => run(i)}
          >
            <span className="palette__group">{c.group}</span>
            {c.iconUrl && <img className="palette__app-icon" src={c.iconUrl} alt="" width={16} height={16} draggable={false} />}
            <span className="palette__label">{c.label}</span>
            {keysFor(c.id)[0] && <kbd className="palette__kbd">{fmt(keysFor(c.id)[0])}</kbd>}
          </li>
        ))}
        {items.length === 0 && <li className="palette__empty">No matching commands</li>}
      </ul>
    </Modal>
  );
}
