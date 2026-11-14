import { IconAlbum, IconAperture, IconCopy, IconFileExport, IconFolderOpen, IconArrowBackUp, IconLock, IconRestore, IconRotate2, IconRotateClockwise2, IconStar, IconTrash, IconX } from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useLocation, useNavigate } from "react-router";
import { openExportSheet } from "@/components/export-sheet";
import { openAlbumPicker } from "@/components/album-picker/store";
import { isMac } from "@/components/window-controls";
import { useSelection } from "@/hooks/use-selection";
import { useTileActions } from "@/hooks/use-tile-actions";
import { useTrash } from "@/hooks/use-trash";
import { openDevelop } from "@/lib/develop-source";
import { moveToHidden, moveToLibrary } from "@/lib/vault";
import "./styles.scss";

/**
 * One action in the bar. The label is a separate element so a narrow window can drop it
 * and leave the icon: `aria-label` and `title` carry the name either way, so the bar
 * never has to scroll or spill off the screen.
 */
function Action({
  label,
  hint,
  icon,
  onClick,
  variant,
}: {
  label: string;
  hint?: string;
  icon: React.ReactNode;
  onClick: () => void;
  /** `icon`: the name only as a tooltip, for the smaller actions. */
  variant?: "quiet" | "danger" | "quiet-danger" | "icon";
}) {
  const cls = ["selection-bar__btn"];
  if (variant === "quiet" || variant === "quiet-danger") cls.push("selection-bar__btn--quiet");
  if (variant === "danger" || variant === "quiet-danger") cls.push("selection-bar__btn--danger");
  if (variant === "icon") cls.push("selection-bar__btn--icon");
  return (
    <button type="button" className={cls.join(" ")} onClick={onClick} aria-label={label} title={hint ? `${label} · ${hint}` : label}>
      {icon}
      <span className="selection-bar__label">{label}</span>
    </button>
  );
}

/** Floating action bar shown while anything is selected (UX spec §8.1). */
export function SelectionBar() {
  const { count, ids, clear } = useSelection();
  const { setFavorite, reveal, trash, copyTo, rotate } = useTileActions();
  const { restore, deleteNow } = useTrash();
  const path = useLocation().pathname;
  const inTrash = path.startsWith("/trash");
  const inHidden = path.startsWith("/hidden");
  const qc = useQueryClient();
  const navigate = useNavigate();
  const list = [...ids];
  const one = count === 1 ? list[0] : null;
  return (
    <AnimatePresence>
      {count > 0 && (
        <motion.div
          className="selection-bar"
          role="toolbar"
          aria-label="Selection"
          initial={{ opacity: 0, y: 24, x: "-50%" }}
          animate={{ opacity: 1, y: 0, x: "-50%" }}
          exit={{ opacity: 0, y: 24, x: "-50%" }}
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
        >
          <span className="selection-bar__count" aria-label={`${count} selected`}>
            <b>{count}</b>
            <span className="selection-bar__label"> selected</span>
          </span>
          {inHidden ? (
            <Action label="Move to Library" icon={<IconArrowBackUp size={16} />} onClick={() => void moveToLibrary(qc, list)} />
          ) : inTrash ? (
            <>
              <Action label="Restore" icon={<IconRestore size={16} />} onClick={() => void restore(list)} />
              <Action label="Delete now" icon={<IconTrash size={16} />} onClick={() => void deleteNow(list)} variant="danger" />
            </>
          ) : (
            <>
              <Action label="Favorite" hint="F" icon={<IconStar size={16} />} onClick={() => void setFavorite(list, true)} />
              <Action label="Add to album" hint="A" icon={<IconAlbum size={16} />} onClick={() => openAlbumPicker(list)} />
              <Action label="Rotate left" hint={`${isMac() ? "⌘" : "Ctrl+"}[`} icon={<IconRotate2 size={16} />} onClick={() => void rotate(list, -1)} variant="icon" />
              <Action label="Rotate right" hint={`${isMac() ? "⌘" : "Ctrl+"}]`} icon={<IconRotateClockwise2 size={16} />} onClick={() => void rotate(list, 1)} variant="icon" />
              <Action label="Copy files to…" icon={<IconCopy size={16} />} onClick={() => void copyTo(list)} />
              <Action label="Export…" icon={<IconFileExport size={16} />} onClick={() => openExportSheet(list)} />
              <Action label="Develop" icon={<IconAperture size={16} />} onClick={() => {
                  openDevelop(navigate, { only: list, label: "Selection" }, list[0]);
                  clear();
                }}
              />
              {one !== null && (
                <Action
                  label={isMac() ? "Reveal in Finder" : "Show in Explorer"}
                  icon={<IconFolderOpen size={16} />}
                  onClick={() => reveal(one)}
                  variant="quiet"
                />
              )}
              <Action label="Move to Hidden" icon={<IconLock size={16} />} onClick={() => void moveToHidden(qc, list)} variant="icon" />
              <Action label="Trash" hint="Delete" icon={<IconTrash size={16} />} onClick={() => void trash(list)} variant="quiet-danger" />
            </>
          )}
          <button type="button" className="selection-bar__btn selection-bar__btn--quiet" onClick={clear} title="Esc" aria-label="Deselect">
            <IconX size={16} />
          </button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
