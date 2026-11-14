import React from "react";
import ReactDOM from "react-dom/client";
import { Providers } from "@/app";
import { AppRouter } from "@/routes/router";
import { isMac } from "@/components/window-controls";
import { startHealth } from "@/lib/health";

// Every screen sizes around the window controls with --window-controls-w (Round 15).
document.documentElement.dataset.mac = String(isMac());

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("@/dev/browser-shim");
}

// The page beats and pings from boot, setup screens included (Round 15).
startHealth();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Providers>
      <AppRouter />
    </Providers>
  </React.StrictMode>,
);
