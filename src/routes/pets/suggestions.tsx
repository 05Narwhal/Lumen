import { Button } from "@mantine/core";
import { IconCheck, IconX } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { toast } from "@/components/toast";
import { useSettings } from "@/hooks/use-settings";
import { invalidateLayouts } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import type { PetPlace } from "@/types/generated/PetPlace";
import type { PetSummary } from "@/types/generated/PetSummary";
import type { Suggestion } from "@/types/generated/Suggestion";
import { mediaUrl } from "@/utils/media";

/**
 * "Is this Milo?" (Round 4C): photos close to this pet but not close enough to join on their
 * own — two look-alikes after a Split, mostly. Answered one by one or all at once, with Undo.
 * A suggestion's `faceId` is the pet detection's id here.
 */
export function PetSuggestions({ pet }: { pet: PetSummary }) {
  const { settings, isLoading } = useSettings();
  const on = !isLoading && settings?.smart?.enabled !== false;
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["pets", "suggestions", pet.id], queryFn: () => invoke<Suggestion[]>("pet_suggestions", { pet: pet.id, limit: 24 }), enabled: on });
  const [answered, setAnswered] = useState<ReadonlySet<number>>(new Set());
  const shown = (q.data ?? []).filter((s) => !answered.has(s.faceId));
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["pets"] }), invalidateLayouts(qc)]), [qc]);
  const who = pet.name ?? `this ${pet.species}`;

  const answer = useCallback(
    async (list: Suggestion[], yes: boolean) => {
      setAnswered((a) => new Set([...a, ...list.map((s) => s.faceId)]));
      const done: { s: Suggestion; place: PetPlace | null }[] = [];
      let failure: unknown = null;
      for (const s of list) {
        try {
          done.push({ s, place: await invoke<PetPlace | null>("answer_pet_suggestion", { detection: s.faceId, pet: pet.id, yes }) });
        } catch (e) {
          failure = e;
          break;
        }
      }
      const unhide = (ids: number[]) => setAnswered((a) => new Set([...a].filter((f) => !ids.includes(f))));
      unhide(list.slice(done.length).map((s) => s.faceId));
      refresh();
      const undo = () =>
        void (async () => {
          try {
            for (const d of done) {
              if (d.place) await invoke("restore_pet_places", { places: [d.place] });
              else await invoke("lift_pet_refusal", { detection: d.s.faceId, pet: pet.id });
            }
          } catch (e) {
            toast({ message: isAppError(e) ? e.message : "The answer could not be undone." });
          }
          unhide(done.map((d) => d.s.faceId));
          refresh();
        })();
      if (failure !== null) {
        toast({ message: isAppError(failure) ? failure.message : "The answer could not be saved.", undo: done.length > 0 ? undo : undefined });
        return;
      }
      const n = list.length === 1 ? "1 photo" : `${list.length} photos`;
      toast({ message: yes ? `Added ${n} to ${who}` : `${n} kept out of ${who}`, undo });
    },
    [pet.id, refresh, who],
  );

  if (!on || shown.length === 0) return null;
  return (
    <section className="pets-route__suggest" aria-label="Suggestions">
      <header className="pets-route__suggest-head">
        <h2>{pet.name ? `Is this ${pet.name}?` : `Is this the same ${pet.species}?`}</h2>
        <Button variant="subtle" size="xs" leftSection={<IconCheck size={14} />} onClick={() => void answer(shown, true)}>
          Yes to all
        </Button>
        <Button variant="subtle" size="xs" leftSection={<IconX size={14} />} onClick={() => void answer(shown, false)}>
          No to all
        </Button>
      </header>
      <ul className="pets-route__suggest-list">
        {shown.map((s) => (
          <li key={s.faceId} className="pets-route__suggest-item">
            <img className="pets-route__face" src={mediaUrl("pet", s.faceId)} alt="" loading="lazy" />
            <span className="pets-route__suggest-answers">
              <button type="button" className="pets-route__answer" data-yes aria-label={`Yes, this is ${pet.name ?? "them"}`} onClick={() => void answer([s], true)}>
                <IconCheck size={14} />
              </button>
              <button type="button" className="pets-route__answer" aria-label={`No, not ${pet.name ?? "them"}`} onClick={() => void answer([s], false)}>
                <IconX size={14} />
              </button>
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
