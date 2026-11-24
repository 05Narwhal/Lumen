import { Button } from "@mantine/core";
import { IconPlug } from "@tabler/icons-react";
import { listen } from "@tauri-apps/api/event";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { pickFile, pickFolder } from "@/lib/dialogs";
import { useEffect, useState } from "react";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { PluginImporter } from "@/types/generated/PluginImporter";
import "./styles.scss";

const KEY = ["plugin-importers"] as const;

function Card({ importer }: { importer: PluginImporter }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isFile = importer.pick === "file";

  const choose = async () => {
    setError(null);
    const picked = isFile
      ? await pickFile({ filters: [{ name: importer.label, extensions: importer.extensions }] })
      : await pickFolder();
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      await invoke<number>("start_plugin_import", { plugin: importer.plugin, chosen: picked });
      // The running import shows in History at once, as the other imports' starts do.
      void qc.invalidateQueries({ queryKey: queryKeys.batches });
    } catch (e) {
      setError(isAppError(e) ? e.message : "Could not start the import.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="plugin-import-card" data-plugin={importer.plugin}>
      <IconPlug size={20} className="plugin-import-card__icon" />
      <div className="plugin-import-card__body">
        <strong>From {importer.label}</strong>
        <span className="plugin-import-card__hint">{importer.hint}</span>
        {error && (
          <span className="plugin-import-card__error" role="alert">
            {error}
          </span>
        )}
      </div>
      <Button size="xs" variant="default" loading={busy} onClick={() => void choose()}>
        {isFile ? "Choose file…" : "Choose folder…"}
      </Button>
    </div>
  );
}

/** One *From <label>* card per enabled importer plugin; nothing when there are none. */
export function PluginImportCards() {
  const qc = useQueryClient();
  const { data } = useQuery({ queryKey: KEY, queryFn: () => invoke<PluginImporter[]>("list_plugin_importers") });
  useEffect(() => {
    const un = listen("plugins://changed", () => void qc.invalidateQueries({ queryKey: KEY }));
    return () => void un.then((f) => f()).catch(() => {});
  }, [qc]);
  if (!data || data.length === 0) return null;
  return (
    <div className="plugin-import-cards">
      {data.map((i) => (
        <Card key={i.plugin} importer={i} />
      ))}
    </div>
  );
}
