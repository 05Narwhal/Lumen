import { Button } from "@mantine/core";
import { IconCheck, IconX } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { toast } from "@/components/toast";
import { useSettings } from "@/hooks/use-settings";
import { invalidateLayouts } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import type { Suggestion } from "@/types/generated/Suggestion";
import type { SuggestionUndo } from "@/types/generated/SuggestionUndo";
import { mediaUrl } from "@/utils/media";

/**
 * "Is this Anna?" (Round 4B): faces close to this person but not close enough to join on
 * their own, each answered with ✓ or ✗, or all at once. Every answer can be undone. Not
 * shown with smart features off.
 */
export function Suggestions({ person }: { person: PersonSummary }) {
  const { settings, isLoading } = useSettings();
  const on = !isLoading && settings?.smart?.enabled !== false;
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: ["people", "suggestions", person.id],
    queryFn: () => invoke<Suggestion[]>("person_suggestions", { person: person.id, limit: 24 }),
    enabled: on,
  });
  const [answered, setAnswered] = useState<ReadonlySet<number>>(new Set());
  const shown = (q.data ?? []).filter((s) => !answered.has(s.faceId));
  // Not awaited: the toast comes at once and the lists catch up behind it (Round 5B).
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["people"] }), invalidateLayouts(qc)]), [qc]);
  const who = person.name ?? "this person";

  const answer = useCallback(
    async (faces: Suggestion[], yes: boolean) => {
      setAnswered((a) => new Set([...a, ...faces.map((s) => s.faceId)]));
      const unhide = (ids: number[]) => setAnswered((a) => new Set([...a].filter((f) => !ids.includes(f))));
      const undos: SuggestionUndo[] = [];
      let failure: unknown = null;
      for (const s of faces) {
        try {
          undos.push(await invoke<SuggestionUndo>("answer_suggestion", { face: s.faceId, person: person.id, yes }));
        } catch (e) {
          failure = e;
          break;
        }
      }
      // Faces not answered come back to the strip.
      unhide(faces.slice(undos.length).map((s) => s.faceId));
      refresh();
      const undo = () =>
        void (async () => {
          try {
            for (const u of undos) await invoke("undo_suggestion", { undo: u });
          } catch (e) {
            toast({ message: isAppError(e) ? e.message : "The answer could not be undone." });
          }
          unhide(undos.map((u) => u.face));
          refresh();
        })();
      if (failure !== null) {
        toast({ message: isAppError(failure) ? failure.message : "The answer could not be saved.", undo: undos.length > 0 ? undo : undefined });
        return;
      }
      const n = faces.length === 1 ? "1 face" : `${faces.length} faces`;
      toast({ message: yes ? `Added ${n} to ${who}` : `${n} kept out of ${who}`, undo });
    },
    [person.id, refresh, who],
  );

  if (!on || shown.length === 0) return null;
  return (
    <section className="people-route__suggest" aria-label="Suggestions">
      <header className="people-route__suggest-head">
        <h2>{person.name ? `Is this ${person.name}?` : "Is this the same person?"}</h2>
        <Button variant="subtle" size="xs" leftSection={<IconCheck size={14} />} onClick={() => void answer(shown, true)}>
          Yes to all
        </Button>
        <Button variant="subtle" size="xs" leftSection={<IconX size={14} />} onClick={() => void answer(shown, false)}>
          No to all
        </Button>
      </header>
      <ul className="people-route__suggest-list">
        {shown.map((s) => (
          <li key={s.faceId} className="people-route__suggest-item">
            <img className="people-route__face" src={mediaUrl("face", s.faceId)} alt="" loading="lazy" />
            <span className="people-route__suggest-answers">
              <button type="button" className="people-route__answer" data-yes aria-label={`Yes, this is ${person.name ?? "them"}`} onClick={() => void answer([s], true)}>
                <IconCheck size={14} />
              </button>
              <button type="button" className="people-route__answer" aria-label={`No, not ${person.name ?? "them"}`} onClick={() => void answer([s], false)}>
                <IconX size={14} />
              </button>
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
