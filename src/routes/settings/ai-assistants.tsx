import { Button, Switch, TextInput } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { toast } from "@/components/toast";
import { useSettingValue, type useSettings } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AiAssistants } from "@/types/generated/AiAssistants";
import type { McpStatus } from "@/types/generated/McpStatus";
import type { Settings } from "@/types/generated/Settings";
import { SettingGroup, SettingRow } from "./row";

type Update = ReturnType<typeof useSettings>["update"];

const OFF: AiAssistants = { enabled: false, port: 47820, token: "" };

/** The server's address for `port`. */
const url = (port: number) => `http://127.0.0.1:${port}/mcp`;

/** What each client needs to reach Lumen (Round 16). */
export function snippets(a: AiAssistants, exe: string | null | undefined): { client: string; id: string; hint: string; text: string }[] {
  return [
    {
      client: "Claude Code",
      id: "claude-code",
      hint: "Connect, or run this once in a terminal.",
      text: `claude mcp add --transport http lumen ${url(a.port)} --header "Authorization: Bearer ${a.token}"`,
    },
    {
      client: "Codex",
      id: "codex",
      hint: "Connect, or add this to ~/.codex/config.toml.",
      text: `[mcp_servers.lumen]\nurl = "${url(a.port)}"\nhttp_headers = { "Authorization" = "Bearer ${a.token}" }`,
    },
    {
      client: "Claude Desktop",
      id: "claude-desktop",
      hint: "Connect, or add this to claude_desktop_config.json. Lumen gives it the key itself.",
      text: JSON.stringify({ mcpServers: { lumen: { command: exe ?? "Lumen", args: ["--mcp"] } } }, null, 2),
    },
  ];
}

/** The status line: off, waiting, who is connected, or why it can't run. */
export function statusLine(s: McpStatus | undefined, enabled: boolean, port: number): string {
  if (!enabled) return "Off. No port is open.";
  if (!s) return "";
  switch (s.state) {
    case "connected":
      return `Connected: ${s.client ?? "an assistant"}.`;
    case "waiting":
      return `Waiting for a connection on port ${port}.`;
    case "port_in_use":
      return `Port ${port} is in use by another program. Choose another one.`;
    case "failed":
      return `Could not start: ${s.message ?? "unknown error"}.`;
    default:
      return "Off. No port is open.";
  }
}

function copy(text: string) {
  void navigator.clipboard?.writeText(text).then(
    () => toast({ message: "Copied" }),
    () => toast({ message: "Could not copy" }),
  );
}

/** Settings → AI assistants (Round 16): the opt-in MCP server for Develop. */
export function AiAssistantsSection({ settings, update }: { settings: Settings; update: Update }) {
  const a = settings.aiAssistants ?? OFF;
  const qc = useQueryClient();
  const [enabled, setEnabled] = useSettingValue(a.enabled);
  const [port, setPort] = useSettingValue(String(a.port));
  const status = useQuery({
    queryKey: [...queryKeys.settings, "mcp-status"],
    queryFn: () => invoke<McpStatus>("mcp_status"),
    refetchInterval: enabled ? 3000 : false,
  });
  // Round 23 A4: the tools enabled plugins add; 0 (and no line) while the switch is off.
  const pluginTools = useQuery({
    queryKey: [...queryKeys.settings, "plugin-tool-count", enabled],
    queryFn: () => invoke<number>("plugin_tool_count"),
    enabled,
  });
  // Invalidated, not refetched: a disabled query (switch off) stays idle.
  useEffect(() => {
    const unlisten = listen("plugins://changed", () => void qc.invalidateQueries({ queryKey: [...queryKeys.settings, "plugin-tool-count"] }));
    return () => void unlisten.then((f) => f()).catch(() => {});
  }, [qc]);
  const toolCount = enabled ? (pluginTools.data ?? 0) : 0;
  const set = (patch: Partial<AiAssistants>) => {
    update((s) => ({ ...s, aiAssistants: { ...(s.aiAssistants ?? OFF), ...patch } }));
    void status.refetch();
  };
  const savePort = (raw: string) => {
    const n = Number(raw);
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      setPort(String(a.port));
      return;
    }
    if (n !== a.port) set({ port: n });
  };
  const connect = (id: string, name: string) => {
    invoke<string>("mcp_connect", { client: id }).then(
      (message) => toast({ message }),
      (e: { message?: string }) => toast({ message: e?.message ?? `Could not connect ${name}` }),
    );
  };
  const newKey = () => {
    void invoke<Settings>("mcp_regenerate_token").then((s) => {
      qc.setQueryData(queryKeys.settings, s);
      toast({ message: "New key made. Copy the setup again into each assistant." });
    });
  };

  return (
    <>
      <SettingGroup>
        <SettingRow
          label="Let AI assistants edit in Develop"
          hint="Photos in Develop's filmstrip are sent to the AI service you connect. Nothing else in your library is visible to it."
        >
          <Switch
            aria-label="Let AI assistants edit in Develop"
            checked={enabled}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setEnabled(v);
              set({ enabled: v });
            }}
          />
        </SettingRow>
        <SettingRow label="Status" hint={statusLine(status.data, enabled, a.port)} />
        {toolCount > 0 && <SettingRow label="Plugins" hint={`Plugins add ${toolCount} ${toolCount === 1 ? "tool" : "tools"}`} />}
        <SettingRow label="Port" hint="Assistants on this computer reach Lumen here. Nothing outside it can.">
          <TextInput
            className="settings__field"
            aria-label="Port"
            inputMode="numeric"
            value={port}
            onChange={(e) => setPort(e.currentTarget.value)}
            onBlur={(e) => savePort(e.currentTarget.value.trim())}
          />
        </SettingRow>
      </SettingGroup>
      {enabled && a.token && (
        <SettingGroup title="Connect an assistant">
          {snippets(a, status.data?.exe).map((s) => (
            <SettingRow key={s.client} label={s.client} hint={s.hint} wide>
              <div className="settings__snippet">
                <pre className="settings__code settings__snippet-text" aria-label={`${s.client} setup`}>
                  {s.text}
                </pre>
                <Button size="xs" variant="default" aria-label={`Connect ${s.client}`} onClick={() => connect(s.id, s.client)}>
                  Connect
                </Button>
                <Button size="xs" variant="default" onClick={() => copy(s.text)}>
                  Copy
                </Button>
              </div>
            </SettingRow>
          ))}
          <SettingRow label="Key" hint="Assistants set up with the old key stop reaching Lumen until you copy the setup again.">
            <Button size="xs" variant="default" onClick={newKey}>
              New key
            </Button>
          </SettingRow>
        </SettingGroup>
      )}
    </>
  );
}
