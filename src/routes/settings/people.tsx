import { Button } from "@mantine/core";
import { IconCheck } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useSyncExternalStore } from "react";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import { usePeopleActions, useRemovedPeople } from "@/hooks/use-people";
import { invoke } from "@/lib/ipc";
import type { RejectedFace } from "@/types/generated/RejectedFace";
import { mediaUrl } from "@/utils/media";
import { SettingGroup, SettingRow } from "./row";

/**
 * The removed people picked on the People page. The inspector's *Restore selected* reads it
 * too, so it lives outside the page's state; the page empties it when it closes.
 */
let picked: ReadonlySet<number> = new Set();
const pickListeners = new Set<() => void>();
const setPicked = (next: ReadonlySet<number> | ((s: ReadonlySet<number>) => ReadonlySet<number>)) => {
  picked = typeof next === "function" ? next(picked) : next;
  pickListeners.forEach((l) => l());
};
const subscribePicked = (l: () => void) => {
  pickListeners.add(l);
  return () => void pickListeners.delete(l);
};
const usePicked = () => useSyncExternalStore(subscribePicked, () => picked);

/** The selection and the restore that empties it, for the page and its inspector. */
export function usePeoplePick() {
  const { removed } = useRemovedPeople();
  const { restore } = usePeopleActions();
  const set = usePicked();
  const live = removed.filter((p) => set.has(p.id));
  const restoreMany = (list: PersonSummary[]) => {
    setPicked(new Set());
    void restore(list);
  };
  return { removed, picked: set, live, setPicked, restoreMany };
}

const TILE = {
  initial: { opacity: 0, scale: 0.96 },
  animate: { opacity: 1, scale: 1 },
  exit: { opacity: 0, scale: 0.9 },
  transition: { duration: 0.18, ease: [0.2, 0, 0, 1] },
} as const;

/**
 * Settings → People (Round 4B; tiles since 2026-11): the way back for someone removed from
 * People. A tile selects on click and shows *Restore* on hover or focus; a selection raises a
 * bar that restores them all. Every restore has an Undo toast. A restored tile fades and
 * shrinks out while the others close the gap.
 */
export function RemovedPeople() {
  const { removed, picked: on, live, restoreMany } = usePeoplePick();
  useEffect(() => () => setPicked(new Set()), []);
  const toggle = (id: number) =>
    setPicked((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  return (
    <AnimatePresence initial={false} mode="wait">
      {removed.length === 0 ? (
        <motion.div key="empty" {...TILE}>
          <SettingGroup title="Removed from People">
            <SettingRow label="No one has been removed from People." hint="Their photos are never touched; restoring someone puts them back." />
          </SettingGroup>
        </motion.div>
      ) : (
        <motion.div key="faces" className="settings-faces" exit={TILE.exit} transition={TILE.transition}>
          <div className="settings-faces__tools">
            <h3 className="settings-group__title">Removed from People</h3>
            <span className="settings-faces__count">
              {removed.length.toLocaleString()} {removed.length === 1 ? "person" : "people"} · click to select, or restore one from its corner
            </span>
            <Button variant="default" size="xs" onClick={() => setPicked(live.length ? new Set() : new Set(removed.map((p) => p.id)))}>
              {live.length ? "Clear" : "Select all"}
            </Button>
          </div>
          <ul className="settings-faces__grid" data-picking={live.length > 0 || undefined}>
            <AnimatePresence mode="popLayout">
              {removed.map((p) => {
                const label = p.name ?? "Unnamed person";
                const sel = on.has(p.id);
                return (
                  <motion.li key={p.id} layout {...TILE} className="settings-faces__tile" data-picked={sel || undefined}>
                    <button type="button" className="settings-faces__pick" aria-pressed={sel} aria-label={`Select ${label}`} onClick={() => toggle(p.id)}>
                      {p.coverFaceId != null ? <img className="settings-faces__face" src={mediaUrl("face", p.coverFaceId)} alt="" loading="lazy" /> : <span className="settings-faces__face" />}
                      <span className="settings-faces__name">{p.name ?? "Unnamed"}</span>
                      <span className="settings-faces__sub">
                        {p.photoCount.toLocaleString()} {p.photoCount === 1 ? "photo" : "photos"}
                      </span>
                    </button>
                    <span className="settings-faces__check" aria-hidden>
                      {sel && <IconCheck size={11} stroke={3} />}
                    </span>
                    <button type="button" className="settings-faces__restore" aria-label={`Restore ${label}`} onClick={() => restoreMany([p])}>
                      Restore
                    </button>
                  </motion.li>
                );
              })}
            </AnimatePresence>
          </ul>
          {live.length > 0 && (
            <div className="settings-faces__bar" role="toolbar" aria-label="Selected people">
              <span>{live.length} selected</span>
              <Button variant="default" size="xs" onClick={() => setPicked(new Set())}>
                Cancel
              </Button>
              <Button size="xs" onClick={() => restoreMany(live)}>
                Restore {live.length}
              </Button>
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Why a detection was set aside, in words. */
const REASONS: Record<string, string> = {
  screen: "on a screen",
  drawing: "a drawing",
  print: "a printed photo",
  statue: "a statue",
  user: "marked Not a person",
};

/**
 * Faces Lumen set aside as not people (Round 4B): the crop check's screens, drawings, prints
 * and statues, and *Not a person*. *This is a real face* brings one back, pinned to nobody.
 */
export function NotFaces() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["people", "rejected"], queryFn: () => invoke<RejectedFace[]>("list_rejected_faces", { limit: 200 }) });
  const faces = q.data ?? [];
  if (faces.length === 0) return null;
  const restore = async (f: RejectedFace) => {
    await invoke("restore_face", { face: f.faceId });
    await qc.invalidateQueries({ queryKey: ["people"] });
  };
  const count = faces.length === 1 ? "1 face set aside" : `${faces.length.toLocaleString()}${faces.length === 200 ? "+" : ""} faces set aside`;
  return (
    <SettingGroup title="Not people">
      <SettingRow label={count} hint="Screens, drawings, prints and statues, and faces you marked Not a person." wide>
        <ul className="settings-faces__grid">
          {faces.map((f) => {
            const why = REASONS[f.reason] ?? f.reason;
            return (
              <li key={f.faceId} className="settings-faces__tile" title={why}>
                <div className="settings-faces__pick">
                  <img className="settings-faces__face" src={mediaUrl("face", f.faceId)} alt="" loading="lazy" />
                  <span className="settings-faces__sub">{why}</span>
                </div>
                <button type="button" className="settings-faces__restore" aria-label={`This is a real face (${why})`} onClick={() => void restore(f)}>
                  Real face
                </button>
              </li>
            );
          })}
        </ul>
      </SettingRow>
    </SettingGroup>
  );
}
