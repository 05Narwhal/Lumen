import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { AiButtonState } from "@/components/ai-button";
import { AI_DELAY_MS, stageLabel } from "@/lib/ai-stages";
import { AI_FINISHED, AI_PROGRESS } from "@/lib/events";
import { invoke } from "@/lib/ipc";
import type { AiFinished } from "@/types/generated/AiFinished";
import type { AiProgress } from "@/types/generated/AiProgress";

export interface AiTask<T> {
  /** null = idle. `visible` flips after the delay, so a fast action never shows anything. */
  state: AiButtonState | null;
  /**
   * Resolves null when cancelled, superseded or ignored because one is already running.
   * `stillWanted` lets `fn` check, before it applies anything, that a cancel did not win.
   */
  run: (fn: (task: string, stillWanted: () => boolean) => Promise<T>) => Promise<T | null>;
  cancel: () => Promise<void>;
}

interface Run {
  id: string;
  cancelled: boolean;
  /** Stops the delay timer and the event listeners; safe to call twice. */
  end: () => void;
}

/**
 * One interactive AI action at a time (Round 12 spec §5): makes the task id, listens to
 * `ai://progress` / `ai://finished` for it *before* the command starts, and exposes a state
 * for `AiButton`. A second `run` while one is going is ignored; the button turns a second
 * click into `cancel`. A result that arrives after `cancel` is discarded.
 */
export function useAiTask<T>(_feature: string): AiTask<T> {
  const [state, setState] = useState<AiButtonState | null>(null);
  const current = useRef<Run | null>(null);

  const release = useCallback((run: Run) => {
    run.end();
    if (current.current === run) {
      current.current = null;
      setState(null);
    }
  }, []);

  // Leaving the screen mid-run stops the listening; the command still finishes on its own.
  useEffect(() => () => current.current?.end(), []);

  const run = useCallback(
    async (fn: (task: string, stillWanted: () => boolean) => Promise<T>): Promise<T | null> => {
      if (current.current) return null;
      const id = crypto.randomUUID();
      const offs: Promise<() => void>[] = [];
      let timer: ReturnType<typeof setTimeout> | undefined;
      let ended = false;
      const r: Run = {
        id,
        cancelled: false,
        end() {
          if (ended) return;
          ended = true;
          clearTimeout(timer);
          offs.forEach((p) => void p.then((off) => off()));
        },
      };
      current.current = r;
      setState({ visible: false, stage: stageLabel("working", 0, 0), done: 0, total: 0 });
      timer = setTimeout(() => {
        if (current.current === r) setState((s) => s && { ...s, visible: true });
      }, AI_DELAY_MS);
      offs.push(
        listen<AiProgress>(AI_PROGRESS, (e) => {
          const p = e.payload;
          if (p.task !== id || current.current !== r) return;
          setState((s) => s && { visible: s.visible, stage: stageLabel(p.stage, p.done, p.total), done: p.done, total: p.total });
        }),
        listen<AiFinished>(AI_FINISHED, (e) => {
          if (e.payload.task === id && e.payload.cancelled) r.cancelled = true;
        }),
      );
      try {
        await Promise.all(offs);
        const result = await fn(id, () => !r.cancelled && current.current === r);
        return r.cancelled ? null : result;
      } finally {
        release(r);
      }
    },
    [release],
  );

  const cancel = useCallback(async () => {
    const r = current.current;
    if (!r) return;
    r.cancelled = true;
    release(r);
    try {
      await invoke<boolean>("ai_cancel", { task: r.id });
    } catch {
      // The backend already finished; there is nothing left to cancel.
    }
  }, [release]);

  return { state, run, cancel };
}
