import { listen } from "@tauri-apps/api/event";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { DENOISE_FINISHED, DENOISE_PROGRESS } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import type { DenoiseFinished } from "@/types/generated/DenoiseFinished";
import type { DenoiseProgress } from "@/types/generated/DenoiseProgress";
import type { DenoiseStatus } from "@/types/generated/DenoiseStatus";

/** No progress for this long while running: "taking longer than usual" (Round 15). */
export const SLOW_MS = 90_000;

export interface DenoiseState {
  status: DenoiseStatus | null;
  /** 0–1 while this photo is being denoised, else null. */
  progress: number | null;
  /** The last run's failure, in words. */
  error: string | null;
  /** Running, and no progress for `SLOW_MS`: say so (the user cancels; the app never gives up). */
  slow: boolean;
  start: () => Promise<void>;
  cancel: () => Promise<void>;
}

/**
 * Denoise for one photo (Phase 8d): whether it can run, its progress from
 * `denoise://progress`, and `onFinished` when `denoise://finished` arrives for it.
 */
export function useDenoise(assetId: number, onFinished: (f: DenoiseFinished) => void): DenoiseState {
  const qc = useQueryClient();
  const key = ["denoise", assetId] as const;
  const status = useQuery({ queryKey: key, queryFn: () => invoke<DenoiseStatus>("denoise_status", { assetId }) });
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [slow, setSlow] = useState(false);
  // Moves on with every progress event, so the slow timer starts over.
  const [tick, setTick] = useState(0);
  const finished = useRef(onFinished);
  finished.current = onFinished;
  const running = progress !== null;

  // Another photo, another run: nothing carries over.
  useEffect(() => {
    setProgress(null);
    setError(null);
  }, [assetId]);

  useEffect(() => {
    setSlow(false);
    if (!running) return;
    const t = setTimeout(() => setSlow(true), SLOW_MS);
    return () => clearTimeout(t);
  }, [running, tick]);

  useEffect(() => {
    const offs = [
      listen<DenoiseProgress>(DENOISE_PROGRESS, (e) => {
        if (e.payload.assetId !== assetId) return;
        setProgress(e.payload.total ? e.payload.done / e.payload.total : 0);
        setTick((t) => t + 1);
      }),
      listen<DenoiseFinished>(DENOISE_FINISHED, (e) => {
        const f = e.payload;
        if (f.assetId !== assetId) return;
        setProgress(null);
        // A cancel is not a failure to show.
        setError(f.ok || f.error?.startsWith("cancelled") ? null : (f.error ?? "Denoise failed."));
        void qc.invalidateQueries({ queryKey: ["denoise", assetId] });
        finished.current(f);
      }),
    ];
    return () => offs.forEach((p) => void p.then((off) => off()));
  }, [assetId, qc]);

  const start = useCallback(async () => {
    setError(null);
    setProgress(0);
    try {
      await invoke("start_denoise", { assetId });
    } catch (e) {
      setProgress(null);
      setError(isAppError(e) ? e.message : "Denoise could not start.");
    }
  }, [assetId]);

  const cancel = useCallback(async () => {
    setProgress(null);
    await invoke("cancel_denoise", { assetId });
  }, [assetId]);

  return { status: status.data ?? null, progress, error, slow, start, cancel };
}
