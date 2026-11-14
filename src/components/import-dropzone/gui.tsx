import { IconDownload } from "@tabler/icons-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { useImport } from "@/hooks/use-import";
import { isAppError } from "@/lib/ipc";
import { isMediaPath } from "@/lib/media-extensions";
import "./styles.scss";


/**
 * What a drop imports: photo and video files exactly as dropped (one batch), and anything
 * else as a folder with its subfolders. Dropping one photo used to import its whole parent
 * folder. Deduplicated; a path that is not a folder is refused by the backend.
 */
export function splitDrop(paths: string[]): { files: string[]; folders: string[] } {
  const files: string[] = [];
  const folders: string[] = [];
  for (const p of paths) {
    const bucket = isMediaPath(p) ? files : folders;
    if (!bucket.includes(p)) bucket.push(p);
  }
  return { files, folders };
}

/** OS drag-and-drop onto the window starts a folder import (UX spec §8.2). */
export function ImportDropzone() {
  const [over, setOver] = useState(false);
  const { startFolderImport, startFilesImport } = useImport();
  const navigate = useNavigate();

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") setOver(true);
        else if (p.type === "leave") setOver(false);
        else if (p.type === "drop") {
          setOver(false);
          const { files, folders } = splitDrop(p.paths);
          if (files.length + folders.length === 0) return;
          Promise.all([
            ...(files.length ? [startFilesImport(files)] : []),
            ...folders.map((path) => startFolderImport({ path, recursive: true })),
          ])
            .then(() => navigate("/import"))
            .catch((e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not start the import." }));
        }
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [startFolderImport, startFilesImport, navigate]);

  return (
    <AnimatePresence>
      {over && (
        <motion.div
          className="import-dropzone"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
        >
          <div className="import-dropzone__card">
            <IconDownload size={32} />
            <span>Drop to import</span>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
