import { useCallback, useEffect, useRef, useState } from "react";
import { Window } from "@/components/window";
import { api, inTauri } from "@/lib/api";
import { previewScreen } from "@/dev/browser-shim";
import { Done } from "@/screens/done";
import { Installed } from "@/screens/installed";
import { Options, type OptionValues } from "@/screens/options";
import { Progress, type TaskView } from "@/screens/progress";
import { DeleteLibraries } from "@/screens/delete-libraries";
import { Remove, type LibraryChoice } from "@/screens/remove";
import { Welcome } from "@/screens/welcome";
import type { BinCheck, Closing, Deleted, FolderCheck, InstallMode, InstallState, LibraryDetail, ModelsOutcome, SetupProgress } from "@/types";

type Screen = "welcome" | "options" | "progress" | "done" | "installed" | "remove" | "delete-libraries";

const idle = (total = 0): TaskView => ({ state: "waiting", done: 0, total });

/** Minutes left from the rate since the download started; undefined until it can be told. */
function secondsLeft(startedAt: number, p: SetupProgress): number | undefined {
  const elapsed = (Date.now() - startedAt) / 1000;
  if (elapsed < 3 || p.done <= 0) return undefined;
  return Math.round(((p.total - p.done) / p.done) * elapsed);
}

export function App() {
  const [state, setState] = useState<InstallState | null>(null);
  const [screen, setScreen] = useState<Screen>("welcome");
  const [values, setValues] = useState<OptionValues>({ desktopShortcut: true, startWithWindows: false, downloadModels: true });
  const [folder, setFolder] = useState("");
  const [check, setCheck] = useState<FolderCheck | null>(null);
  const [heading, setHeading] = useState("Installing Lumen");
  const [lumen, setLumen] = useState<TaskView>({ state: "running", done: 0, total: 1 });
  const [models, setModels] = useState<TaskView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<ModelsOutcome>("later");
  const [openNow, setOpenNow] = useState(true);
  const [removeData, setRemoveData] = useState(false);
  const [removePhase, setRemovePhase] = useState<"confirm" | "removing" | "done">("confirm");
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [closing, setClosing] = useState<Closing | null>(null);
  const [libraryDetails, setLibraryDetails] = useState<LibraryDetail[] | null>(null);
  const [deleteChoice, setDeleteChoice] = useState<LibraryChoice | null>(null);
  const [tooBig, setTooBig] = useState<{ paths: string[]; check: BinCheck } | null>(null);
  const [binChecking, setBinChecking] = useState(false);
  const [deleted, setDeleted] = useState<Deleted[]>([]);
  const modelsStart = useRef(0);
  const lastOptions = useRef<{ mode: InstallMode }>({ mode: "install" });

  // Who is already here decides the first screen; the shim's ?screen= decides in a browser.
  useEffect(() => {
    void api.getInstallState().then((s) => {
      setState(s);
      setFolder(s.defaultFolder);
      const preview = inTauri() ? null : previewScreen();
      if (preview) {
        setScreen(preview.screen);
        if (preview.screen === "progress") {
          setLumen({ state: "done", done: 1, total: 1 });
          setModels({ state: "running", done: Math.round(500 * 1024 ** 2 * preview.fraction), total: 500 * 1024 ** 2, secondsLeft: 70 });
        }
        if (preview.screen === "done") setOutcome(new URLSearchParams(location.search).get("models") === "later" ? "later" : "complete");
        if (preview.screen === "remove") setRemovePhase("confirm");
        if (preview.screen === "delete-libraries") void api.libraryDetails().then(setLibraryDetails);
      } else {
        setScreen(s.removeOnly ? "remove" : s.installed.kind === "none" ? "welcome" : "installed");
      }
    });
  }, []);

  useEffect(() => {
    let off = () => {};
    void api.onProgress((p) => {
      if (p.task === "lumen") {
        setLumen({ state: p.done >= p.total ? "done" : "running", done: p.done, total: p.total });
      } else {
        if (modelsStart.current === 0) modelsStart.current = Date.now();
        setModels({ state: p.done >= p.total ? "done" : "running", done: p.done, total: p.total, secondsLeft: secondsLeft(modelsStart.current, p) });
      }
    }).then((u) => { off = u; });
    return () => off();
  }, []);

  // A running Lumen closing first: "Closing Lumen…" or its import question, until it has gone.
  useEffect(() => {
    let off = () => {};
    void api.onLumen((c) => setClosing(c === "closed" ? null : c)).then((u) => { off = u; });
    return () => off();
  }, []);

  // Cancel while Lumen closes: stop waiting, end Setup, change nothing.
  const cancelWait = useCallback(() => {
    void api.cancelWait().then(() => api.close());
  }, []);

  // Check the folder whenever it or the space the install needs changes.
  useEffect(() => {
    if (!state || screen !== "options") return;
    let live = true;
    setCheck(null);
    void api.checkFolder(folder, state.needBytes + (values.downloadModels ? state.modelsBytes : 0)).then((c) => live && setCheck(c));
    return () => { live = false; };
  }, [state, folder, values.downloadModels, screen]);

  const run = useCallback(
    async (mode: InstallMode, opts: OptionValues, into: string) => {
      if (!state) return;
      lastOptions.current = { mode };
      modelsStart.current = 0;
      setError(null);
      setHeading(mode === "update" ? "Updating Lumen" : mode === "repair" ? "Repairing Lumen" : "Installing Lumen");
      setLumen({ state: "running", done: 0, total: 1 });
      setModels(opts.downloadModels ? idle(state.modelsBytes) : null);
      setScreen("progress");
      try {
        const result = await api.install({ folder: into, ...opts, mode });
        setOutcome(result.models);
        setLumen((l) => ({ ...l, state: "done" }));
        setModels((m) => (m ? { ...m, state: result.models === "complete" ? "done" : "later" } : m));
        setScreen("done");
      } catch (e) {
        setLumen((l) => ({ ...l, state: "failed" }));
        setError(typeof e === "string" ? e : "The installer stopped before it finished. Nothing was changed; try again.");
      }
    },
    [state],
  );

  if (!state) return <Window><span /></Window>;

  const progressLight = screen === "progress" ? 0.45 + 0.55 * (lumen.state === "done" ? 0.5 + 0.5 * (models && models.total ? models.done / models.total : 1) : (lumen.done / Math.max(1, lumen.total)) * 0.5) : undefined;

  switch (screen) {
    case "welcome":
      return (
        <Window center light={0.45}>
          <Welcome
            version={state.ours}
            sizeOnDisk={state.sizeOnDisk}
            onInstall={() => void run("install", values, folder)}
            onOptions={() => setScreen("options")}
          />
        </Window>
      );
    case "options":
      return (
        <Window light={0.5}>
          <Options
            folder={folder}
            check={check}
            {...values}
            modelsBytes={state.modelsBytes}
            needBytes={state.needBytes}
            onChange={(patch) => setValues((v) => ({ ...v, ...patch }))}
            onChangeFolder={() => void api.chooseFolder(folder).then((p) => p && setFolder(p))}
            onBack={() => setScreen("welcome")}
            onInstall={() => void run("install", values, folder)}
          />
        </Window>
      );
    case "progress":
      return (
        <Window light={progressLight}>
          <Progress
            heading={heading}
            lumen={lumen}
            models={models}
            error={error}
            onOpen={() => void api.openLumen()}
            onSkip={() => { void api.skipModels(); setModels((m) => (m ? { ...m, state: "later" } : m)); }}
            onRetry={() => void run(lastOptions.current.mode, values, state.installed.kind === "none" ? folder : (state.installed as { location: string }).location)}
            closing={closing}
            onCancel={cancelWait}
          />
        </Window>
      );
    case "done":
      return (
        <Window center light={1}>
          <Done models={outcome} openNow={openNow} onToggleOpen={setOpenNow} onFinish={() => { if (openNow) void api.openLumen(); void api.close(); }} />
        </Window>
      );
    case "installed":
      if (state.installed.kind === "none") return null;
      return (
        <Window light={0.45}>
          <Installed
            installed={state.installed}
            ours={state.ours}
            lumenRunning={state.lumenRunning}
            onPrimary={() => void run(state.installed.kind === "older" ? "update" : "repair", { desktopShortcut: true, startWithWindows: false, downloadModels: true }, (state.installed as { location: string }).location)}
            onOpen={() => void api.openLumen()}
            onRemove={() => setScreen("remove")}
          />
        </Window>
      );
    case "remove":
      return (
        <Window title="Remove Lumen" light={0.3}>
          <Remove
            libraries={state.libraries}
            dataBytes={state.appDataBytes}
            safetyCopies={state.safetyCopies}
            removeData={removeData}
            phase={removePhase}
            onToggleData={setRemoveData}
            closing={closing}
            error={removeError}
            deleteChoice={deleteChoice}
            deleted={deleted}
            onDeleteLibraries={() => {
              setLibraryDetails(null);
              setTooBig(null);
              setScreen("delete-libraries");
              void api.libraryDetails().then(setLibraryDetails);
            }}
            onUndoDelete={() => setDeleteChoice(null)}
            onCancel={() => {
              if (removePhase === "removing") cancelWait();
              else if (state.removeOnly) void api.close();
              else setScreen(state.installed.kind === "none" ? "welcome" : "installed");
            }}
            onRemove={() => {
              setRemoveError(null);
              setRemovePhase("removing");
              api.uninstall(removeData, deleteChoice && { paths: deleteChoice.paths, mode: deleteChoice.mode }).then(
                (d) => {
                  setDeleted(d);
                  setRemovePhase("done");
                },
                (e) => {
                  setRemovePhase("confirm");
                  setRemoveError(typeof e === "string" ? e : "Lumen could not be removed. Lumen is still installed; try again.");
                },
              );
            }}
            onFinish={() => void api.close()}
          />
        </Window>
      );
    case "delete-libraries": {
      const choose = (paths: string[], mode: LibraryChoice["mode"]) => {
        const names = paths.map((p) => libraryDetails?.find((l) => l.path === p)?.name ?? p);
        setDeleteChoice({ paths, names, mode });
        setTooBig(null);
        setScreen("remove");
      };
      return (
        <Window title="Remove Lumen" light={0.3}>
          <DeleteLibraries
            libraries={libraryDetails}
            checking={binChecking}
            tooBig={tooBig?.check ?? null}
            onBack={() => setScreen("remove")}
            onConfirm={(paths) => {
              setBinChecking(true);
              void api.checkRecycleBin(paths).then((check) => {
                setBinChecking(false);
                if (check.fits) choose(paths, "recycle");
                else setTooBig({ paths, check });
              });
            }}
            onEraseAnyway={() => tooBig && choose(tooBig.paths, "erase")}
            onCancelTooBig={() => setTooBig(null)}
          />
        </Window>
      );
    }
  }
}
