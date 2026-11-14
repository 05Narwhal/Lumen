import { useEffect, useRef } from "react";
import { useStripIds } from "@/hooks/use-develop-strip";
import { developSourceStore, isEmptySource, lastDevelopPhoto, DEFAULT_DEVELOP_SOURCE } from "@/lib/develop-source";
import { invoke } from "@/lib/ipc";

/**
 * Tells Rust what AI assistants may see (Round 16): Develop's filmstrip and the photo open in
 * it. Mounted at the root so it holds while the user is elsewhere. Off, the strip is not even
 * listed, and Rust's scope is emptied if it was ever filled.
 */
export function useMcpScope(enabled: boolean) {
  const source = developSourceStore.use();
  const open = lastDevelopPhoto.use();
  // Off, ask for the empty strip: nothing is fetched.
  const { data: ids, isSuccess } = useStripIds(enabled ? source : DEFAULT_DEVELOP_SOURCE);
  const settled = enabled && (isSuccess || isEmptySource(source));
  const key = settled ? JSON.stringify(ids ?? []) : "";
  const sent = useRef(false);
  useEffect(() => {
    if (enabled ? !settled : !sent.current) return;
    const strip = enabled ? (ids ?? []) : [];
    sent.current = enabled;
    invoke("mcp_set_scope", { ids: strip, open: open !== null && strip.includes(open) ? open : null }).catch(() => {});
    // `key` stands for `ids`, whose identity changes with every refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, settled, key, open]);
}
