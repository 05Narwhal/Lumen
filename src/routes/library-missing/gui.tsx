import { Button } from "@mantine/core";
import { IconLockExclamation, IconPlugConnectedX } from "@tabler/icons-react";
import { pickFolder } from "@/lib/dialogs";
import { useState } from "react";
import { LibraryList } from "@/components/library-list";
import { toast, Toaster } from "@/components/toast";
import { useLibraries } from "@/hooks/use-libraries";
import { isAppError } from "@/lib/ipc";
import type { LibraryEntry } from "@/types/generated/LibraryEntry";
import type { LibraryProblem } from "@/types/generated/LibraryProblem";
import "./styles.scss";
import { useWindowDrag } from "@/hooks/use-window-drag";

/**
 * Shown instead of the app when the current library is there but can't be opened (Round 17): it
 * is newer than this Lumen, or its safety copy failed. It was left exactly as it was.
 */
export function LibraryProblemRoute({ problem }: { problem: LibraryProblem }) {
  const { openLibrary } = useLibraries();
  const [others, setOthers] = useState(false);
  const [trying, setTrying] = useState(false);

  const tryAgain = async () => {
    setTrying(true);
    try {
      await openLibrary(problem.path);
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not open the library." });
    } finally {
      setTrying(false);
    }
  };

  const drag = useWindowDrag();
  return (
    <div className="library-missing" {...drag}>
      <div className="library-missing__card" role="alert" data-no-drag>
        <IconLockExclamation size={36} className="library-missing__icon" />
        <h1>This library can't be opened</h1>
        <p>{problem.message}</p>
        <p>
          <code>{problem.path}</code>
        </p>
        <div className="library-missing__actions">
          <Button loading={trying} onClick={() => void tryAgain()}>
            Try again
          </Button>
          <Button variant="default" onClick={() => setOthers((v) => !v)} aria-expanded={others}>
            Choose another library
          </Button>
        </div>
        {others && (
          <div className="library-missing__list">
            <LibraryList />
          </div>
        )}
      </div>
      <Toaster />
    </div>
  );
}

/** Shown instead of the app when the current library's disk is not connected (UX spec §2). */
export function LibraryMissingRoute({ entry }: { entry: LibraryEntry }) {
  const { locateLibrary, createLibrary } = useLibraries();
  const [others, setOthers] = useState(false);

  const locate = async () => {
    const dir = await pickFolder({ title: `Locate “${entry.name}”` });
    if (typeof dir !== "string") return;
    try {
      await locateLibrary({ id: entry.id, path: dir });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not open the library." });
    }
  };

  const createNew = async () => {
    const dir = await pickFolder({ title: "Choose a folder for the new library" });
    if (typeof dir !== "string") return;
    try {
      await createLibrary(dir);
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not create the library." });
    }
  };

  const drag = useWindowDrag();
  return (
    <div className="library-missing" {...drag}>
      <div className="library-missing__card" role="alert" data-no-drag>
        <IconPlugConnectedX size={36} className="library-missing__icon" />
        <h1>Library not found</h1>
        <p>
          <strong>{entry.name}</strong> was last seen at <code>{entry.path}</code>. Connect the drive and it opens by itself, or point
          Lumen at its new location.
        </p>
        <div className="library-missing__actions">
          <Button onClick={locate}>Locate…</Button>
          <Button variant="default" onClick={() => setOthers((v) => !v)} aria-expanded={others}>
            Choose another library
          </Button>
          <Button variant="subtle" onClick={createNew}>
            Create new library
          </Button>
        </div>
        {others && (
          <div className="library-missing__list">
            <LibraryList />
          </div>
        )}
      </div>
      <Toaster />
    </div>
  );
}
