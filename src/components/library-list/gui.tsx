import { Button } from "@mantine/core";
import { IconCheck, IconPlugConnected, IconPlugConnectedX } from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "@/components/toast";
import { useLibraries } from "@/hooks/use-libraries";
import { isAppError } from "@/lib/ipc";
import type { LibraryStatus } from "@/types/generated/LibraryStatus";
import "./styles.scss";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** Every library this install knows about, with Open / Locate / Forget (UX spec §2). */
export function LibraryList() {
  const { listing, current, openLibrary, locateLibrary, forgetLibrary, createLibrary, existsAt } = useLibraries();
  const libraries = listing?.libraries ?? [];

  const locate = async (s: LibraryStatus) => {
    const dir = await open({ directory: true, multiple: false, title: `Locate “${s.entry.name}”` });
    if (typeof dir !== "string") return;
    try {
      await locateLibrary({ id: s.entry.id, path: dir });
      toast({ message: `Now using ${dir}` });
    } catch (e) {
      fail(e, "Could not open the library.");
    }
  };

  const openOther = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose a library folder" });
    if (typeof dir !== "string") return;
    try {
      if (!(await existsAt(dir))) {
        toast({ message: "No Lumen library in that folder." });
        return;
      }
      await openLibrary(dir);
      toast({ message: `Now using ${dir}` });
    } catch (e) {
      fail(e, "Could not open the library.");
    }
  };

  const createNew = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose a folder for the new library" });
    if (typeof dir !== "string") return;
    try {
      await createLibrary(dir);
      toast({ message: `Created a library at ${dir}` });
    } catch (e) {
      fail(e, "Could not create the library.");
    }
  };

  return (
    <div className="library-list">
      <ul className="library-list__rows">
        {libraries.map((s) => {
          const isCurrent = current?.id === s.entry.id;
          return (
            <li key={s.entry.id} className="library-list__row" data-current={isCurrent} data-available={s.available}>
              <span className="library-list__status" aria-hidden>
                {isCurrent ? <IconCheck size={16} /> : s.available ? <IconPlugConnected size={16} /> : <IconPlugConnectedX size={16} />}
              </span>
              <div className="library-list__text">
                <span className="library-list__name">{s.entry.name}</span>
                <code className="library-list__path">{s.entry.path}</code>
              </div>
              <span className="library-list__pill">{isCurrent ? "Open" : s.available ? "Available" : "Not connected"}</span>
              <div className="library-list__actions">
                {!isCurrent && s.available && (
                  <Button size="xs" onClick={() => openLibrary(s.entry.path).catch((e) => fail(e, "Could not open the library."))}>
                    Open
                  </Button>
                )}
                {!s.available && (
                  <Button size="xs" variant="default" onClick={() => locate(s)}>
                    Locate…
                  </Button>
                )}
                {!isCurrent && (
                  <Button
                    size="xs"
                    variant="subtle"
                    onClick={() =>
                      forgetLibrary(s.entry.id)
                        .then(() => toast({ message: `Forgot “${s.entry.name}”. The folder is untouched.` }))
                        .catch((e) => fail(e, "Could not forget the library."))
                    }
                  >
                    Forget
                  </Button>
                )}
              </div>
            </li>
          );
        })}
        {libraries.length === 0 && <li className="library-list__none">No libraries yet.</li>}
      </ul>
      <div className="library-list__footer">
        <Button variant="default" onClick={openOther}>
          Open other…
        </Button>
        <Button variant="default" onClick={createNew}>
          Create new…
        </Button>
      </div>
    </div>
  );
}
