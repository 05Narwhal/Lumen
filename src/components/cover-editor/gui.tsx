import { Button, ColorPicker, Modal, SegmentedControl } from "@mantine/core";
import { IconColorPicker } from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useState, type CSSProperties } from "react";
import { CoverStack } from "@/components/cover-stack";
import { COVER_SHAPES, IconCover, SWATCHES, coverColor } from "@/components/icon-cover";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { CoverStyle } from "@/types/generated/CoverStyle";
import { IconPicker } from "./icon-picker";
import { closeCoverEditor, coverEditorStore, type CoverTarget } from "./store";
import "./styles.scss";

/** Saves a cover; the albums query (collections are under it) refetches. */
export async function saveCoverStyle(qc: ReturnType<typeof useQueryClient>, target: Pick<CoverTarget, "kind" | "id">, style: CoverStyle | null) {
  if (target.kind === "album") await invoke<void>("set_album_cover_style", { albumId: target.id, style });
  else await invoke<void>("set_collection_cover_style", { id: target.id, style });
  await qc.invalidateQueries({ queryKey: queryKeys.albums });
}

const HEX = /^#[0-9a-f]{6}$/i;
/** Where *Your own colour* starts: data, like any hex the person then picks, not a theme colour. */
const FIRST_CUSTOM = "#4f7cff";

/**
 * Choose how an album or collection looks (Julian's 2026-11-08 list, item 3): its photos, or a
 * shape (folder, book, box, binder) with one of two dozen icons on it, in one of ten swatches or
 * a colour of one's own. Saving is reversible, so it offers Undo rather than asking.
 */
export function CoverEditor({ target, onClose }: { target: CoverTarget; onClose: () => void }) {
  const qc = useQueryClient();
  const [mode, setMode] = useState<"photos" | "icon">(target.style ? "icon" : "photos");
  const [draft, setDraft] = useState<CoverStyle>(
    target.style ?? { kind: "icon", shape: target.kind === "album" ? "book" : "folder", icon: "photo", color: "swatch-6" },
  );
  const custom = HEX.test(draft.color);
  const [picking, setPicking] = useState(custom);
  const set = (patch: Partial<Omit<CoverStyle, "kind">>) => setDraft((d) => ({ ...d, ...patch }));

  const save = async () => {
    const next = mode === "icon" ? draft : null;
    const before = target.style;
    onClose();
    try {
      await saveCoverStyle(qc, target, next);
      toast({ message: "Cover changed", undo: () => void saveCoverStyle(qc, target, before) });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not change the cover." });
    }
  };

  return (
    <Modal opened onClose={onClose} title={`Cover for “${target.name}”`} centered size="lg" classNames={{ content: "cover-editor__modal" }}>
      <div className="cover-editor">
        <div className="cover-editor__preview" data-mode={mode}>
          {mode === "icon" ? (
            <IconCover style={draft} />
          ) : target.photos.length > 0 ? (
            <CoverStack items={target.photos} />
          ) : (
            <span className="cover-editor__none">No photos yet</span>
          )}
        </div>
        <div className="cover-editor__controls">
          <SegmentedControl
            fullWidth
            value={mode}
            onChange={(v) => setMode(v as "photos" | "icon")}
            data={[
              { label: "Photos", value: "photos" },
              { label: "Icon", value: "icon" },
            ]}
          />
          {mode === "icon" && (
            <>
              <fieldset className="cover-editor__group">
                <legend>Shape</legend>
                <div className="cover-editor__shapes">
                  {COVER_SHAPES.map(({ shape, label }) => (
                    <button
                      key={shape}
                      type="button"
                      className="cover-editor__shape"
                      aria-label={label}
                      aria-pressed={draft.shape === shape}
                      title={label}
                      onClick={() => set({ shape })}
                    >
                      <IconCover style={{ ...draft, shape }} />
                    </button>
                  ))}
                </div>
              </fieldset>
              <fieldset className="cover-editor__group">
                <legend>Icon</legend>
                <IconPicker value={draft.icon} onChange={(icon) => set({ icon })} />
              </fieldset>
              <fieldset className="cover-editor__group">
                <legend>Colour</legend>
                <div className="cover-editor__swatches">
                  {SWATCHES.map((s, i) => (
                    <button
                      key={s}
                      type="button"
                      className="cover-editor__swatch"
                      aria-label={`Colour ${i + 1}`}
                      aria-pressed={draft.color === s}
                      style={{ "--swatch": coverColor(s) } as CSSProperties}
                      onClick={() => {
                        set({ color: s });
                        setPicking(false);
                      }}
                    />
                  ))}
                  <button
                    type="button"
                    className="cover-editor__swatch cover-editor__swatch--custom"
                    aria-label="Your own colour"
                    aria-pressed={custom}
                    title="Your own colour"
                    style={custom ? ({ "--swatch": draft.color } as CSSProperties) : undefined}
                    onClick={() => {
                      setPicking(true);
                      if (!custom) set({ color: FIRST_CUSTOM });
                    }}
                  >
                    {!custom && <IconColorPicker size={14} />}
                  </button>
                </div>
                {picking && custom && (
                  <ColorPicker className="cover-editor__picker" format="hex" value={draft.color} onChange={(color) => HEX.test(color) && set({ color })} fullWidth />
                )}
              </fieldset>
            </>
          )}
        </div>
      </div>
      <div className="cover-editor__actions">
        <Button variant="default" onClick={onClose}>
          Cancel
        </Button>
        <Button onClick={() => void save()}>Save</Button>
      </div>
    </Modal>
  );
}

/** The editor for covers changed from a menu or the palette. Mount once. */
export function CoverEditorHost() {
  const { target } = coverEditorStore.use();
  if (!target) return null;
  return <CoverEditor key={`${target.kind}${target.id}`} target={target} onClose={closeCoverEditor} />;
}
