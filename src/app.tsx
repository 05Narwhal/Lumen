import "@mantine/core/styles.css";
import "@fontsource-variable/inter";
import "@fontsource-variable/fraunces";
import "@/styles/globals.scss";

import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { MotionConfig } from "motion/react";
import { useEffect, useState, type ReactNode } from "react";
import { useDateOrder } from "@/hooks/use-date-order";
import { useSettings } from "@/hooks/use-settings";
import { useApplyTheme } from "@/hooks/use-theme";
import { startEventBridge } from "@/lib/events";
import { reportVideoSupport } from "@/lib/video-support";
import { cssVariablesResolver, mantineTheme } from "@/styles/mantine-theme";

function ThemeBridge({ children }: { children: ReactNode }) {
  useApplyTheme();
  return <>{children}</>;
}

/**
 * Motion follows Lumen's own "Reduce motion" setting, not the OS.
 *
 * `reducedMotion="user"` read the OS preference, which meant Framer Motion (the shell,
 * cards, viewer, toasts) could be switched off by a Windows visual-effects toggle while
 * our CSS tokens — driven by the app setting — kept animating. Two authorities for one
 * decision, and the app offered no way to turn the Framer half back on. The setting is
 * now the only authority, so what the switch says is what the app does.
 */
function MotionBridge({ children }: { children: ReactNode }) {
  const { settings } = useSettings();
  // Every date shown follows the date order (Round 11).
  useDateOrder();
  return <MotionConfig reducedMotion={settings?.appearance.reduceMotion ? "always" : "never"}>{children}</MotionConfig>;
}

/** Subscribes to Tauri events for the lifetime of the app. */
function EventBridge() {
  const qc = useQueryClient();
  useEffect(() => {
    // What this WebView plays, told to Rust once (Round 8A).
    void reportVideoSupport();
    let stop: (() => void) | undefined;
    let cancelled = false;
    startEventBridge(qc).then((s) => {
      if (cancelled) s();
      else stop = s;
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [qc]);
  return null;
}

export function Providers({ children }: { children: ReactNode }) {
  const [client] = useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } }),
  );
  return (
    <QueryClientProvider client={client}>
      {/* Under Vitest, Mantine's transitions render at once: in jsdom a menu could otherwise
          stay `display: none` when its animation frame came late, failing tests at random. */}
      <MantineProvider theme={mantineTheme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="auto" env={import.meta.env.MODE === "test" ? "test" : "default"}>
        <MotionBridge>
          <EventBridge />
          <ThemeBridge>{children}</ThemeBridge>
        </MotionBridge>
      </MantineProvider>
    </QueryClientProvider>
  );
}
