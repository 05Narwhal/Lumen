import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { useDevelopSession } from "@/hooks/use-develop-session";
import { IDENTITY } from "@/hooks/use-edit-session";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { McpRequest } from "@/types/generated/McpRequest";
import type { Recipe } from "@/types/generated/Recipe";
import { MCP_REQUEST, useMcpBridge } from "./mcp-bridge";

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
const warmer: Recipe = { ...IDENTITY, develop: { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, temperature: 15 } } };
const ask = (req: Partial<McpRequest> & Pick<McpRequest, "op">) => emitMock(MCP_REQUEST, { req: 1, assetId: 7, label: "AI: warmer", ...req });

function mount(open = vi.fn()) {
  return renderHook(
    () => {
      const s = useDevelopSession(7);
      useMcpBridge(s, open);
      return s;
    },
    { wrapper: Providers },
  );
}

describe("useMcpBridge", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_develop_draft", () => null);
    mockCommand("develop_preview", () => ({ token: "1", width: 3, height: 2, superseded: false, histogram: null }));
    mockCommand("push_history", () => 1);
    mockCommand("save_develop_draft", () => 1);
    mockCommand("hold_close", () => null);
    mockCommand("mcp_set_mounted", () => null);
    mockCommand("mcp_reply", () => null);
    mockCommand("mcp_transform", () => warmer);
  });

  it("says Develop is mounted, and no longer once it closes", async () => {
    const { unmount } = mount();
    await waitFor(() => expect(calls("mcp_set_mounted")).toEqual([{ mounted: true }]));
    unmount();
    expect(calls("mcp_set_mounted").at(-1)).toEqual({ mounted: false });
  });

  it("applies a change on the draft on screen as one step, and undo takes back only its own", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, contrast: 0.3 } })));
    await act(async () => ask({ op: { kind: "patch", patch: { basic: { temperature: 15 } } } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(1));
    expect((calls("mcp_transform")[0].recipe as Recipe).develop?.basic.contrast).toBe(0.3);
    expect(calls("mcp_reply")[0]).toEqual({ req: 1, reply: { recipe: warmer, error: null, message: null } });
    expect(result.current.develop.basic.temperature).toBe(15);
    await waitFor(() => expect(calls("push_history").at(-1)).toMatchObject({ label: "AI: warmer" }));

    await act(async () => ask({ req: 2, op: { kind: "undo" } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(2));
    expect(calls("mcp_reply")[1]).toMatchObject({ req: 2, reply: { error: null } });
    expect(result.current.develop.basic.temperature).toBe(0);
    await act(async () => ask({ req: 3, op: { kind: "undo" } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(3));
    expect(calls("mcp_reply")[2]).toMatchObject({ req: 3, reply: { error: "last_step_not_ai" } });
  });

  it("refuses another photo as busy, opens one when asked, and passes Rust's refusals on", async () => {
    const open = vi.fn();
    const { result } = mount(open);
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => ask({ assetId: 8, op: { kind: "patch", patch: {} } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(1));
    expect(calls("mcp_reply")[0]).toMatchObject({ reply: { error: "develop_busy" } });
    await act(async () => ask({ req: 2, assetId: 8, op: { kind: "open" } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(2));
    expect(open).toHaveBeenCalledWith(8);
    mockCommand("mcp_transform", () => Promise.reject({ code: "invalid_patch", message: "bad" }));
    await act(async () => ask({ req: 3, op: { kind: "patch", patch: { x: 1 } } }));
    await waitFor(() => expect(calls("mcp_reply")).toHaveLength(3));
    expect(calls("mcp_reply")[2]).toEqual({ req: 3, reply: { recipe: null, error: "invalid_patch", message: "bad" } });
  });
});
