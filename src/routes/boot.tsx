import { useEffect, useRef, useState, type ReactNode } from "react";
import { Splash } from "@/components/splash";
import { useLibraries } from "@/hooks/use-libraries";
import { hydrateDevelopSource, rememberDevelopSource } from "@/lib/develop-source-persist";
import { invoke } from "@/lib/ipc";
import { useSettings } from "@/hooks/use-settings";
import { LibraryMissingRoute, LibraryProblemRoute } from "@/routes/library-missing";

const MIN_SPLASH_MS = 600;
// Dev screenshots (headless browsers freeze the exit animation): skip the splash entirely.
const shouldSkipSplash = () =>
  import.meta.env.DEV && (window as unknown as { __LUMEN_NO_SPLASH__?: boolean }).__LUMEN_NO_SPLASH__ === true;

/**
 * Gate: nothing routes until settings and the library registry are known, so the
 * setup redirect can never fire on stale data. The splash stays up at least briefly
 * so it reads as a deliberate screen rather than a flash. A current library whose
 * disk is missing gets the Locate screen instead of the app, and one that can't be opened
 * (newer than this Lumen, or no safety copy, Round 17) says why. After *Restart Lumen* (Round 15)
 * it opens where the person was (`resume`), with no splash.
 */
export function Boot({ children, resume }: { children: ReactNode; resume?: (to: string) => void }) {
  const { settings } = useSettings();
  const { isLoaded, current, missing, problem } = useLibraries();
  const [skipSplash, setSkipSplash] = useState(shouldSkipSplash);
  const [minElapsed, setMinElapsed] = useState(skipSplash);
  // `undefined` until asked: nothing routes before the router is on the saved screen.
  const [resumed, setResumed] = useState<boolean>();
  useEffect(() => {
    const t = setTimeout(() => setMinElapsed(true), MIN_SPLASH_MS);
    return () => clearTimeout(t);
  }, []);
  useEffect(() => {
    let live = true;
    invoke<string | null>("take_resume")
      .catch(() => null)
      .then((to) => {
        if (!live) return;
        if (to) {
          resume?.(to);
          setSkipSplash(true);
          setMinElapsed(true);
        }
        setResumed(!!to);
      });
    return () => {
      live = false;
    };
    // Once per launch: the saved screen is read once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Round 21: when each gate opened, in ms since the page began, so the next slow start after a
  // reload names the gate that held it. Logged once per gate.
  const logged = useRef(new Set<string>());
  useEffect(() => {
    const gate = (name: string, open: boolean) => {
      if (!open || logged.current.has(name)) return;
      logged.current.add(name);
      void invoke("log_boot_gate", { gate: name, ms: Math.round(performance.now()) }).catch(() => {});
    };
    gate("settings", !!settings);
    gate("library", isLoaded);
    gate("resume", resumed !== undefined);
  }, [settings, isLoaded, resumed]);
  // Round 21 §13: Develop's last filmstrip is put back before any route renders, then written
  // back as it changes. `current` is the open library, whose strip it is.
  const [stripBack, setStripBack] = useState(false);
  const libraryId = current?.id ?? null;
  useEffect(() => {
    if (!isLoaded) return;
    if (libraryId === null) {
      setStripBack(true);
      return;
    }
    let stop: (() => void) | undefined;
    let live = true;
    void hydrateDevelopSource().then(() => {
      if (!live) return;
      stop = rememberDevelopSource();
      setStripBack(true);
    });
    return () => {
      live = false;
      stop?.();
    };
  }, [isLoaded, libraryId]);
  useEffect(() => {
    if (stripBack) void invoke("log_boot_gate", { gate: "strip", ms: Math.round(performance.now()) }).catch(() => {});
  }, [stripBack]);
  const ready = !!settings && isLoaded && minElapsed && resumed !== undefined && stripBack;
  return (
    <>
      <Splash visible={!ready && !skipSplash} />
      {ready && (current === null && problem ? <LibraryProblemRoute problem={problem} /> : current === null && missing ? <LibraryMissingRoute entry={missing} /> : children)}
    </>
  );
}
