import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invalidateLayouts } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AssetFace } from "@/types/generated/AssetFace";
import type { FacePlace } from "@/types/generated/FacePlace";
import type { MergeUndo } from "@/types/generated/MergeUndo";
import type { PersonGroup } from "@/types/generated/PersonGroup";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import type { PersonUndo } from "@/types/generated/PersonUndo";
import type { UntagUndo } from "@/types/generated/UntagUndo";

/** Everyone the face models have grouped, named first (Phase 7b); removed people are not. */
export function usePeople() {
  const q = useQuery({ queryKey: queryKeys.people, queryFn: () => invoke<PersonSummary[]>("list_people") });
  const { refetch } = q;
  const retry = useCallback(() => void refetch(), [refetch]);
  // `isError` only before any data: a failed refetch keeps showing the list it had.
  return { people: q.data ?? [], isLoading: q.isLoading, isError: q.isError && q.data === undefined, retry };
}

/** The people removed from People (Round 4B), for Settings → People. */
export function useRemovedPeople() {
  const q = useQuery({ queryKey: queryKeys.removedPeople, queryFn: () => invoke<PersonSummary[]>("list_removed_people") });
  return { removed: q.data ?? [], isLoading: q.isLoading };
}

/** The face of `person` in a photo, for *Use as main photo* on a photo. */
export async function faceOf(person: number, assetId: number): Promise<number | null> {
  const faces = await invoke<AssetFace[]>("faces_for_asset", { id: assetId });
  return faces.find((f) => f.personId === person)?.id ?? null;
}

/** Naming, merging, removing and covers. Everything reversible offers Undo. */
export function usePeopleActions() {
  const qc = useQueryClient();
  // Not awaited: the change is made and its toast shown at once; the lists catch up behind it
  // (Round 5B: awaiting the refetch held every People edit for as long as it took).
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["people"] }), invalidateLayouts(qc)]), [qc]);

  /** The new name shows at once, and goes back if the library could not save it. */
  const name = useCallback(
    async (id: number, value: string) => {
      const trimmed = value.trim();
      await qc.cancelQueries({ queryKey: queryKeys.people });
      const before = qc.getQueryData<PersonSummary[]>(queryKeys.people);
      qc.setQueryData<PersonSummary[]>(queryKeys.people, (list) => list?.map((p) => (p.id === id ? { ...p, name: trimmed || null } : p)));
      try {
        await invoke("name_person", { id, name: value });
      } catch (e) {
        qc.setQueryData(queryKeys.people, before);
        toast({ message: isAppError(e) ? e.message : "Could not save the name." });
        return;
      }
      refresh();
    },
    [qc, refresh],
  );

  const merge = useCallback(
    async (into: PersonSummary, from: PersonSummary) => {
      const undo = await invoke<MergeUndo>("merge_people", { into: into.id, from: from.id });
      refresh();
      toast({
        message: `Merged into ${into.name ?? from.name ?? "one person"}`,
        undo: () => void invoke("restore_person", { undo }).then(refresh),
      });
    },
    [refresh],
  );

  /** *Remove from People*: a real person, not wanted in People. */
  /** *Remove from People* for one person or a selection, with one Undo for all of them. */
  const remove = useCallback(
    async (who: PersonSummary | PersonSummary[]) => {
      const list = Array.isArray(who) ? who : [who];
      for (const p of list) await invoke("remove_person", { id: p.id });
      refresh();
      toast({
        message: list.length === 1 ? `${list[0].name ?? "This person"} removed from People` : `${list.length} people removed from People`,
        undo: () => void Promise.all(list.map((p) => invoke("restore_removed_person", { id: p.id }))).then(refresh),
      });
    },
    [refresh],
  );

  /** Settings → People's *Restore*, for one person or a selection, with one Undo for all. */
  const restore = useCallback(
    async (who: PersonSummary | PersonSummary[]) => {
      const list = Array.isArray(who) ? who : [who];
      for (const p of list) await invoke("restore_removed_person", { id: p.id });
      refresh();
      toast({
        message: list.length === 1 ? `${list[0].name ?? "This person"} is back in People` : `${list.length} people are back in People`,
        undo: () => void Promise.all(list.map((p) => invoke("remove_person", { id: p.id }))).then(refresh),
      });
    },
    [refresh],
  );

  /** *Not a person*: these were never faces. */
  const reject = useCallback(
    async (who: PersonSummary | PersonSummary[]) => {
      const list = Array.isArray(who) ? who : [who];
      const undos: PersonUndo[] = [];
      for (const p of list) undos.push(await invoke<PersonUndo>("reject_person", { id: p.id }));
      refresh();
      toast({
        message: list.length === 1 ? "Not a person: those faces are gone" : `Not people: ${list.length} removed with their faces`,
        undo: () => void Promise.all(undos.map((undo) => invoke("undo_reject_person", { undo }))).then(refresh),
      });
    },
    [refresh],
  );

  /** *Use as main photo*: `face` becomes `person`'s picture. */
  const setCover = useCallback(
    async (person: number, face: number) => {
      const before = await invoke<number | null>("set_person_cover", { person, face });
      refresh();
      toast({
        message: "Main photo changed",
        undo: before == null ? undefined : () => void invoke("set_person_cover", { person, face: before }).then(refresh),
      });
    },
    [refresh],
  );

  /** `person` is not in these photos: they leave them, for `to` or for nobody. */
  const movePhotos = useCallback(
    async (person: PersonSummary, assetIds: number[], to: PersonSummary | null) => {
      const places = await invoke<FacePlace[]>("move_person_photos", { person: person.id, assetIds, to: to?.id ?? null });
      refresh();
      const n = assetIds.length === 1 ? "1 photo" : `${assetIds.length} photos`;
      toast({
        message: to ? `Moved ${n} to ${to.name ?? "another person"}` : `Removed ${n} from ${person.name ?? "this person"}`,
        undo: () => void invoke("restore_faces", { places }).then(refresh),
      });
    },
    [refresh],
  );

  return { name, merge, remove, restore, reject, setCover, movePhotos };
}

/** Who a face is, chosen in the person picker: someone listed, or a new person by name. */
export type TagTarget = { person: PersonSummary } | { newName: string };

const whoOf = (t: TagTarget) => ("person" in t ? (t.person.name ?? "this person") : t.newName.trim());

/**
 * Tagging by hand in the viewer (Round 4I): a found face, or a box the user drew. Every tag
 * has a 6 s Undo; an error is told in the core's words.
 */
export function useFaceTags() {
  const qc = useQueryClient();
  // Not awaited: the change is made and its toast shown at once; the lists catch up behind it
  // (Round 5B: awaiting the refetch held every People edit for as long as it took).
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["people"] }), invalidateLayouts(qc)]), [qc]);
  const fail = useCallback((e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not tag that face." }), []);

  /** An existing face is `target`. */
  const tag = useCallback(
    async (face: AssetFace, target: TagTarget) => {
      try {
        if ("person" in target) {
          const place = await invoke<FacePlace>("tag_face", { face: face.id, person: target.person.id });
          refresh();
          toast({ message: `Tagged ${whoOf(target)}`, undo: () => void invoke("restore_faces", { places: [place] }).then(refresh) });
        } else {
          const [person, place] = await invoke<[number, FacePlace]>("create_person_with_face", { face: face.id, name: target.newName.trim() });
          refresh();
          toast({ message: `Tagged ${whoOf(target)}`, undo: () => void invoke("undo_create_person", { person, place }).then(refresh) });
        }
      } catch (e) {
        fail(e);
      }
    },
    [refresh, fail],
  );

  /** A box drawn on `assetId` (0..1 of the unedited photo) is `target`. */
  const addBox = useCallback(
    async (assetId: number, rect: [number, number, number, number], target: TagTarget) => {
      try {
        const [face, created] = await invoke<[number, number | null]>("add_face_box", {
          assetId,
          rect,
          personId: "person" in target ? target.person.id : null,
          newName: "person" in target ? null : target.newName.trim(),
        });
        refresh();
        toast({
          message: `Tagged ${whoOf(target)}`,
          undo: () =>
            void invoke<UntagUndo>("untag_face", { face })
              .then(() => (created != null ? invoke("undo_create_person", { person: created, place: { face, person: null, pinned: false } }) : undefined))
              .then(refresh),
        });
      } catch (e) {
        fail(e);
      }
    },
    [refresh, fail],
  );

  /** *Not {name}*: this face is not who it was given to. */
  const untag = useCallback(
    async (face: AssetFace) => {
      try {
        const undo = await invoke<UntagUndo>("untag_face", { face: face.id });
        refresh();
        toast({ message: `Not ${face.name ?? "them"}`, undo: () => void invoke("undo_untag_face", { undo }).then(refresh) });
      } catch (e) {
        fail(e);
      }
    },
    [refresh, fail],
  );

  return { tag, addBox, untag };
}

const GROUPS_KEY = ["people", "groups"] as const;

/** Manual groups of people (Family, Friends, …): each with the listed people in it. */
export function usePersonGroups() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: GROUPS_KEY, queryFn: () => invoke<PersonGroup[]>("list_person_groups") });
  const refresh = useCallback(() => qc.invalidateQueries({ queryKey: GROUPS_KEY }), [qc]);
  const fail = (e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not change the group." });

  const create = useCallback(
    async (name: string, people: number[] = []) => {
      try {
        const id = await invoke<number>("create_person_group", { name });
        if (people.length) await invoke("set_person_group_members", { group: id, people, member: true });
        refresh();
        return id;
      } catch (e) {
        fail(e);
        return null;
      }
    },
    [refresh],
  );
  const rename = useCallback(
    (id: number, name: string) => invoke("rename_person_group", { id, name }).then(refresh, fail),
    [refresh],
  );
  const remove = useCallback(
    async (g: PersonGroup) => {
      const members = await invoke<number[]>("delete_person_group", { id: g.id });
      refresh();
      toast({ message: `Deleted ${g.name}`, undo: () => void create(g.name, members) });
    },
    [refresh, create],
  );
  const setMember = useCallback(
    (group: number, people: number[], member: boolean) =>
      invoke("set_person_group_members", { group, people, member }).then(refresh, fail),
    [refresh],
  );
  const { refetch } = q;
  const retry = useCallback(() => void refetch(), [refetch]);
  // Round 15: until the groups are known, People draws nobody (never everyone ungrouped).
  return { groups: q.data ?? [], isLoading: q.isPending, isError: q.isError && q.data === undefined, retry, create, rename, remove, setMember };
}
