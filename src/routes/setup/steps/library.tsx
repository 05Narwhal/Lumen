import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { toast } from "@/components/toast";
import { useLibraries } from "@/hooks/use-libraries";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AppInfo } from "@/types/generated/AppInfo";
import { formatBytes } from "@/utils/format";

export function LibraryStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const { createLibrary, openLibrary, existsAt } = useLibraries();
  const info = useQuery({
    queryKey: queryKeys.appInfo,
    queryFn: () => invoke<AppInfo>("get_app_info"),
    staleTime: Infinity,
  });
  const [path, setPath] = useState<string | null>(null);
  const effective = path ?? info.data?.defaultLibraryDir ?? "";
  const free = useQuery({
    queryKey: ["free-space", effective],
    queryFn: () => invoke<number>("get_free_space", { path: effective }),
    enabled: !!effective,
  });
  const exists = useQuery({
    queryKey: ["library-exists", effective],
    queryFn: () => existsAt(effective),
    enabled: !!effective,
  });
  const [busy, setBusy] = useState(false);

  const pick = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose a library folder" });
    if (typeof dir === "string") setPath(dir);
  };

  const go = async () => {
    setBusy(true);
    try {
      if (exists.data) await openLibrary(effective);
      else await createLibrary(effective);
      onNext();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not set up the library." });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="setup__step">
      <h1 className="setup__title">Where should your library live?</h1>
      <p className="setup__lead">
        Originals, thumbnails and the catalogue are stored in one folder you can back up with any tool, or carry on an external
        drive between machines.
      </p>
      <div className="setup__path">
        <code className="setup__path-value">{effective}</code>
        <Button variant="default" onClick={pick}>
          Change…
        </Button>
      </div>
      <p className="setup__hint">
        {free.data !== undefined ? `${formatBytes(free.data)} free` : " "}
        {exists.data ? " · An existing Lumen library was found here." : ""}
      </p>
      <div className="setup__actions">
        <Button variant="subtle" onClick={onBack}>
          Back
        </Button>
        <Button loading={busy} disabled={!effective} onClick={go}>
          {exists.data ? "Open existing library" : "Create library"}
        </Button>
      </div>
    </div>
  );
}
