import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { AppError } from "@/types/generated/AppError";

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

/** Typed `invoke` that always rejects with an `AppError`. */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return (await tauriInvoke(cmd, args)) as T;
  } catch (e) {
    if (isAppError(e)) throw e;
    const err: AppError = {
      code: "unknown",
      message: "Something went wrong.",
      detail: typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e),
    };
    throw err;
  }
}
