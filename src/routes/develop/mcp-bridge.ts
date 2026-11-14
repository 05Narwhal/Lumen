import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { invoke, isAppError } from "@/lib/ipc";
import type { McpReply } from "@/types/generated/McpReply";
import type { McpRequest } from "@/types/generated/McpRequest";
import type { Recipe } from "@/types/generated/Recipe";

export const MCP_REQUEST = "mcp://request";

const refusal = (error: string, message: string): McpReply => ({ recipe: null, error, message });

/**
 * AI assistants' changes to the photo open in Develop (Round 16): Rust sends them here while
 * Develop is on screen, so they land on the draft shown — the user's own edits kept — as one
 * history step, and the sliders move. Every request gets an answer; Rust says `develop_busy`
 * after 10 s without one.
 */
export function useMcpBridge(session: DevelopSession, open: (id: number) => void) {
  const latest = useRef({ session, open });
  latest.current = { session, open };

  useEffect(() => {
    void invoke("mcp_set_mounted", { mounted: true }).catch(() => {});
    const answer = async (req: McpRequest): Promise<McpReply> => {
      const { session: s, open: go } = latest.current;
      if (req.op.kind === "open") {
        go(req.assetId);
        return { recipe: null, error: null, message: null };
      }
      if (req.assetId !== s.assetId || !s.loaded) {
        return refusal("develop_busy", "Another photo is opening in Develop. Try again.");
      }
      if (req.op.kind === "undo") {
        const now = s.undoAi();
        return now ? { recipe: now, error: null, message: null } : refusal("last_step_not_ai", "The last step is the user's, not yours; it was left alone.");
      }
      const next = await invoke<Recipe>("mcp_transform", { assetId: req.assetId, recipe: s.draft, op: req.op });
      // The photo changed while Rust worked: this change is not the new photo's.
      const cur = latest.current.session;
      if (cur.assetId !== req.assetId || !cur.applyExternal(next, req.label)) {
        return refusal("develop_busy", "Another photo opened in Develop meanwhile. Try again.");
      }
      return { recipe: next, error: null, message: null };
    };
    const un = listen<McpRequest>(MCP_REQUEST, (e) => {
      const req = e.payload;
      void answer(req)
        .catch((err): McpReply => (isAppError(err) ? refusal(err.code, err.message) : refusal("internal", String(err))))
        .then((reply) => invoke("mcp_reply", { req: req.req, reply }))
        .catch(() => {});
    });
    return () => {
      void un.then((f) => f());
      void invoke("mcp_set_mounted", { mounted: false }).catch(() => {});
    };
  }, []);
}
