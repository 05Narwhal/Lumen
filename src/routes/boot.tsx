import { useEffect, useRef, useState, type ReactNode } from "react";
import { RestartAlert } from "@/components/restart-alert";
import { Splash } from "@/components/splash";
import { useLibraries } from "@/hooks/use-libraries";
import { hydrateDevelopSource, rememberDevelopSource } from "@/lib/develop-source-persist";
import { invoke } from "@/lib/ipc";
import { useSettings } from "@/hooks/use-settings";
import { LibraryMissingRoute, LibraryProblemRoute } from "@/routes/library-missing";
import type { ResumeInfo } from "@/types/generated/ResumeInfo";

const MIN_SPLASH_MS = 600;
/** Round 24 §6.3: the splash never holds the app longer than this. */
export const STUCK_AFTER_MS = 10_000;
// Dev screenshots (headless browsers freeze the exit animation): skip the splash entirely.
const shouldSkipSplash = () =>
  import.meta.env.DEV && (window as unknown as { __LUMEN_NO_SPLASH__?: boolean }).__LUMEN_NO_SPLASH__ === true;

/**
 * Gate: nothing routes until settings and the library registry are known, so the
 * setup redirect can never fire on stale data. The splash stays up at least briefly
 * so it reads as a deliberate screen rather than a flash. A current library whose
 * disk is missing gets the Locate screen instead of the app, and one that can't be opened
 * (newer than this Lumen, or no safety copy, Round 17) says why. After a crash, a watchdog restart or
 * *Restart Lumen* it opens Home with an alert whose button goes back to where the person was
 * (Round 24 §6.2; `resume` navigates). Past `STUCK_AFTER_MS` the app shell shows with what is
 * ready (a slow strip or resume answer no longer holds it), and if settings or the library are
 * still missing the splash says which, instead of waiting in silence (§6.3).
 */
export function Boot({ children, resume }: { children: ReactNode; resume?: (to: string) => void }) {
  const { settings, error: settingsError } = useSettings();
  const { isLoaded, current, missing, problem } = useLibraries();
  const [skipSplash, setSkipSplash] = useState(shouldSkipSplash);
  const [minElapsed, setMinElapsed] = useState(skipSplash);
  // `undefined` until asked: nothing routes before the router is on the saved screen.
  const [resumed, setResumed] = useState<boolean>();
  // Where the person was before a crash or restart, for the alert's button.
  const [alert, setAlert] = useState<{ route: string | null }>();
  const [stuck, setStuck] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setMinElapsed(true), MIN_SPLASH_MS);
    const s = setTimeout(() => setStuck(true), STUCK_AFTER_MS);
    return () => {
      clearTimeout(t);
      clearTimeout(s);
    };
  }, []);
  useEffect(() => {
    let live = true;
    invoke<ResumeInfo>("resume_info")
      .catch(() => null)
      .then((info) => {
        if (!live) return;
        const relaunched = !!info && info.reason !== "normal";
        if (relaunched) {
          // Home, never the saved screen: a crash in Develop must not take Lumen straight back in.
          resume?.("/");
          setAlert({ route: info.route });
          if (info.reason === "restart") {
            setSkipSplash(true);
            setMinElapsed(true);
          }
        }
        setResumed(relaunched);
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
  const essential = !!settings && isLoaded;
  // Past the ceiling only settings and the library are waited for: the rest is optional polish.
  const ready = essential && (stuck || (minElapsed && resumed !== undefined && stripBack));
  const waitingFor = [!settings && "settings", !isLoaded && "the library list"].filter(Boolean).join(" and ");
  const note =
    stuck && !essential
      ? `Lumen is taking longer than usual to start: still waiting for ${waitingFor}.${settingsError ? ` ${String((settingsError as { message?: string }).message ?? settingsError)}` : ""}`
      : undefined;
  return (
    <>
      <Splash visible={!ready && !skipSplash} note={note} />
      {ready && (current === null && problem ? <LibraryProblemRoute problem={problem} /> : current === null && missing ? <LibraryMissingRoute entry={missing} /> : children)}
      {ready && alert && (
        <RestartAlert
          route={alert.route}
          onBack={(to) => {
            resume?.(to);
            setAlert(undefined);
          }}
          onDismiss={() => setAlert(undefined)}
        />
      )}
    </>
  );
}
