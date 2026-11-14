import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { confirm } from "@/components/confirm-dialog";
import { CLOSE_REQUESTED } from "@/lib/events";
import { invoke } from "@/lib/ipc";

/** Ask what to do when the window is closed during an import. The dialog decides; Rust closes. */
export async function askToClose(): Promise<void> {
  const stop = await confirm({
    title: "An import is still running",
    body: "Closing now stops it. Files already copied are safe, and the rest will be offered again next time.",
    confirmLabel: "Stop import and close",
    cancelLabel: "Keep Lumen open",
    danger: true,
    focusCancel: true,
  });
  if (stop) await invoke("confirm_close");
}

/** Mount once in the shell: answers Rust's `app://close-requested`. */
export function CloseGuard() {
  useEffect(() => {
    let open = false;
    const un = listen(CLOSE_REQUESTED, () => {
      if (open) return;
      open = true;
      void askToClose().finally(() => (open = false));
    });
    return () => void un.then((f) => f());
  }, []);
  return null;
}
