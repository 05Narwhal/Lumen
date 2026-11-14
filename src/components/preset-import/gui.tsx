import { Modal } from "@mantine/core";
import { createStore } from "@/lib/store";
import type { PresetImport } from "@/types/generated/PresetImport";
import "./styles.scss";

const store = createStore<{ report: PresetImport | null }>({ report: null });

/** Opens the list of what a Lightroom preset import could not take (the toast's *Details*). */
export function openPresetNotes(report: PresetImport) {
  store.set({ report });
}

/** What each imported Lightroom preset had that Lumen could not convert. Mount once. */
export function PresetNotesHost() {
  const { report } = store.use();
  const close = () => store.set({ report: null });
  return (
    <Modal opened={report !== null} onClose={close} title="Settings not converted" centered size="md">
      {report && (
        <ul className="preset-notes__list">
          {report.notConverted.map((p) => (
            <li key={p.name}>
              <div className="preset-notes__name">{p.name}</div>
              <div className="preset-notes__settings">{p.settings.join(", ")}</div>
            </li>
          ))}
          {report.failed.map((f) => (
            <li key={f.file}>
              <div className="preset-notes__name">{f.file}</div>
              <div className="preset-notes__settings">{f.reason}</div>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
