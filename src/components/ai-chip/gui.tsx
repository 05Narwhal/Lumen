import { Popover } from "@mantine/core";
import { IconSparkles } from "@tabler/icons-react";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { useSettings } from "@/hooks/use-settings";
import type { McpActivity } from "@/types/generated/McpActivity";
import "./styles.scss";

export const MCP_STATUS = "mcp://status";
/** The chip stays this long after an assistant's last call. */
export const AI_CHIP_LINGER_MS = 60_000;

/** `true` while running, else when the last call ended; `null` before any. */
function useMcpActivity() {
  const [state, setState] = useState<{ client: string | null; running: number; at: number } | null>(null);
  useEffect(() => {
    const un = listen<McpActivity>(MCP_STATUS, (e) => {
      const a = e.payload;
      // Calls can overlap (a `view` while an `edit` renders): count them.
      setState((s) => ({ client: a.client ?? s?.client ?? null, running: Math.max(0, (s?.running ?? 0) + (a.running ? 1 : -1)), at: Date.now() }));
    });
    return () => void un.then((f) => f());
  }, []);
  // Hide once the assistant has been quiet for a minute.
  const [, tick] = useState(0);
  useEffect(() => {
    if (!state || state.running > 0) return;
    const t = setTimeout(() => tick((n) => n + 1), Math.max(0, state.at + AI_CHIP_LINGER_MS - Date.now()));
    return () => clearTimeout(t);
  }, [state]);
  if (!state) return null;
  if (state.running === 0 && Date.now() - state.at >= AI_CHIP_LINGER_MS) return null;
  return { client: state.client ?? "An AI assistant", running: state.running > 0 };
}

/**
 * Develop's top bar while an AI assistant works on the filmstrip (Round 16): "`<client>` is
 * editing", with a spinner while a tool runs; its popover turns AI assistants off.
 */
export function AiChip() {
  const activity = useMcpActivity();
  const { settings, update } = useSettings();
  if (!activity || !settings?.aiAssistants?.enabled) return null;
  const label = `${activity.client} is editing`;
  return (
    <Popover position="bottom" withArrow shadow="md" width={260}>
      <Popover.Target>
        <button type="button" className="ai-chip" aria-label={label} data-running={activity.running}>
          {activity.running ? <span className="ai-chip__spinner" role="progressbar" aria-label="Working" /> : <IconSparkles size={14} aria-hidden />}
          <span className="ai-chip__label">{label}</span>
        </button>
      </Popover.Target>
      <Popover.Dropdown className="ai-chip__dropdown">
        <p className="ai-chip__text">Its edits are drafts in this filmstrip: each is a history step named “AI: …” that you can undo. It sets a rating, label or flag only when you ask.</p>
        <button
          type="button"
          className="ai-chip__off"
          onClick={() => update((s) => ({ ...s, aiAssistants: { ...s.aiAssistants, enabled: false } }))}
        >
          Turn off
        </button>
      </Popover.Dropdown>
    </Popover>
  );
}
