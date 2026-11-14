import { IconTextRecognition } from "@tabler/icons-react";
import { useMemo, useState } from "react";
import type { MouseEvent } from "react";
import { toast } from "@/components/toast";
import { allText, copyText, joinSentences } from "@/lib/photo-text";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";

/** How many sentences the inset shows before *Show all*. */
const INSET_SENTENCES = 2;

/**
 * The info panel's *Text* row (Round 18 C2): a head line (icon, *Text*, the line count), an
 * inset with the first two sentences of what was read, and the text's actions under it:
 * *Show all* opens every line in place (it scrolls, and becomes *Show less*), *Copy* copies
 * them all, *On photo* toggles the overlay (T). While the photo is read for the first time it
 * says *Reading…*; without text the row is absent. The text is selectable, and its context
 * menu (`onMenu`) has *Copy* and *Search for “…”*.
 */
export function TextSection({
  lines,
  reading,
  shown,
  onShowOnPhoto,
  onMenu,
}: {
  lines: ShownTextLine[];
  reading: boolean;
  /** The overlay on the photo is on. */
  shown: boolean;
  onShowOnPhoto: () => void;
  onMenu: (text: string | null, e: MouseEvent) => void;
}) {
  const [all, setAll] = useState(false);
  const sentences = useMemo(() => joinSentences(lines), [lines]);
  if (!lines.length && !reading) return null;
  const copyAll = async () => toast({ message: (await copyText(allText(lines))) ? "Copied" : "Could not copy the text." });
  const menu = (text: string) => (e: MouseEvent) => {
    e.preventDefault();
    onMenu(window.getSelection?.()?.toString().trim() || text, e);
  };
  const more = sentences.length > INSET_SENTENCES;
  return (
    <div className="info-panel__group info-panel__text">
      <div className="info-panel__text-head">
        <IconTextRecognition size={16} aria-hidden />
        <span className="info-panel__label">Text</span>
        {lines.length > 0 && <span className="info-panel__text-count">{lines.length === 1 ? "1 line" : `${lines.length} lines`}</span>}
      </div>
      {lines.length === 0 ? (
        <p className="info-panel__faces-empty">Reading…</p>
      ) : (
        <>
          {all ? (
            <ol className="info-panel__text-inset info-panel__text-all" data-testid="text-inset">
              {lines.map((l) => (
                <li key={l.ord} onContextMenu={menu(l.text)}>
                  {l.text}
                </li>
              ))}
            </ol>
          ) : (
            <div className="info-panel__text-inset" data-testid="text-inset">
              {sentences.slice(0, INSET_SENTENCES).map((s, i) => (
                <p key={i} onContextMenu={menu(s)}>
                  {s}
                </p>
              ))}
            </div>
          )}
          <div className="info-panel__text-actions">
            {(more || all) && (
              <button type="button" className="info-panel__text-action" aria-expanded={all} onClick={() => setAll((v) => !v)}>
                {all ? "Show less" : "Show all"}
              </button>
            )}
            <button type="button" className="info-panel__text-action" onClick={() => void copyAll()}>
              Copy
            </button>
            <button type="button" className="info-panel__text-action" aria-pressed={shown} onClick={onShowOnPhoto}>
              On photo <kbd>T</kbd>
            </button>
          </div>
        </>
      )}
    </div>
  );
}
