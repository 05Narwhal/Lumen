import { createStore } from "@/lib/store";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  message: string;
  action?: ToastAction;
  durationMs: number;
}

export const toastStore = createStore<{ toasts: Toast[] }>({ toasts: [] });

const MAX = 3;
let next = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function dismissToast(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
  toastStore.set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
}

/**
 * Show a toast. Reversible actions pass `undo` (rendered as an Undo button); any other
 * follow-up passes `action`. Shown for 6 s by default.
 */
export function toast(opts: { message: string; undo?: () => void; action?: ToastAction; durationMs?: number }): number {
  const action = opts.action ?? (opts.undo ? { label: "Undo", run: opts.undo } : undefined);
  const t: Toast = { id: next++, message: opts.message, action, durationMs: opts.durationMs ?? 6000 };
  toastStore.set((s) => {
    const all = [...s.toasts, t];
    const dropped = all.slice(0, Math.max(0, all.length - MAX));
    dropped.forEach((d) => {
      clearTimeout(timers.get(d.id));
      timers.delete(d.id);
    });
    return { toasts: all.slice(-MAX) };
  });
  timers.set(t.id, setTimeout(() => dismissToast(t.id), t.durationMs));
  return t.id;
}
