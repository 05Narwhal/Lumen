import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invalidateLayouts } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AssetPet } from "@/types/generated/AssetPet";
import type { NotAPetUndo } from "@/types/generated/NotAPetUndo";
import type { PetMergeUndo } from "@/types/generated/PetMergeUndo";
import type { PetPlace } from "@/types/generated/PetPlace";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import type { PetSummary } from "@/types/generated/PetSummary";

/** Every pet the scan has grouped, named first (Round 4C); removed pets are not. */
export function usePets() {
  const q = useQuery({ queryKey: queryKeys.pets, queryFn: () => invoke<PetSummary[]>("list_pets") });
  return { pets: q.data ?? [], isLoading: q.isLoading };
}

/** Pets taken off the Pets page, for Settings → People. */
export function useRemovedPets() {
  const q = useQuery({ queryKey: queryKeys.removedPets, queryFn: () => invoke<PetSummary[]>("list_removed_pets") });
  return { removed: q.data ?? [], isLoading: q.isLoading };
}

/** "Milo", or "this cat" / "this dog" for a pet with no name yet. */
export const petName = (p: Pick<PetSummary, "name" | "species">) => p.name ?? `this ${p.species}`;

/** People's picker speaks People's shape; a pet's cover is its detection. */
/**
 * The merge picker's groups (Round 9A): `own`'s species first, the other under *Cats* or
 * *Dogs*. A dog can be merged into a cat when the detector got one wrong.
 */
export function bySpecies(pets: PetSummary[], own: string) {
  const species = new Map(pets.map((p) => [p.id, p.species]));
  return (p: PersonSummary) => {
    const s = species.get(p.id) ?? own;
    return s === own ? { order: 0, label: null } : { order: 1, label: s === "cat" ? "Cats" : s === "dog" ? "Dogs" : s };
  };
}

/** *Merged into Milo*, and across species *Merged Rex into Milo (now a cat)*. */
export function mergedMessage(into: PetSummary, from: PetSummary): string {
  if (into.species === from.species) return `Merged into ${into.name ?? from.name ?? `one ${into.species}`}`;
  const fromName = from.name ?? `this ${from.species}`;
  return into.name ? `Merged ${fromName} into ${into.name} (now a ${into.species})` : `Merged ${fromName} into the unnamed ${into.species}`;
}

export const asPicker = (p: PetSummary): PersonSummary => ({ id: p.id, name: p.name, faceCount: p.detectionCount, photoCount: p.photoCount, coverFaceId: p.coverDetectionId });

/** The detection of `pet` in a photo, for *Use as main photo* on a photo. */
export async function detectionOf(pet: number, assetId: number): Promise<number | null> {
  const here = await invoke<AssetPet[]>("asset_pets", { asset: assetId });
  return here.find((d) => d.petId === pet)?.id ?? null;
}

/** Naming, merging, splitting and the rest. Everything reversible offers Undo; nothing asks. */
export function usePetActions() {
  const qc = useQueryClient();
  // Not awaited: the toast comes at once and the lists catch up behind it, as People's do.
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["pets"] }), invalidateLayouts(qc)]), [qc]);
  const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

  /** The new name shows at once, and goes back if the library could not save it. */
  const name = useCallback(
    async (pet: number, value: string) => {
      const trimmed = value.trim();
      await qc.cancelQueries({ queryKey: queryKeys.pets });
      const before = qc.getQueryData<PetSummary[]>(queryKeys.pets);
      qc.setQueryData<PetSummary[]>(queryKeys.pets, (list) => list?.map((p) => (p.id === pet ? { ...p, name: trimmed || null } : p)));
      try {
        await invoke("name_pet", { pet, name: value });
      } catch (e) {
        qc.setQueryData(queryKeys.pets, before);
        fail(e, "Could not save the name.");
        return;
      }
      refresh();
    },
    [qc, refresh],
  );

  const merge = useCallback(
    async (into: PetSummary, from: PetSummary) => {
      try {
        const undo = await invoke<PetMergeUndo>("merge_pets", { into: into.id, from: from.id });
        refresh();
        toast({ message: mergedMessage(into, from), undo: () => void invoke("undo_merge_pets", { undo }).then(refresh) });
      } catch (e) {
        fail(e, "Could not merge them.");
      }
    },
    [refresh],
  );

  /** *These are a different pet*: the photos become a new pet. Returns its id. */
  const split = useCallback(
    async (pet: PetSummary, assetIds: number[]) => {
      try {
        const [newPet, places] = await invoke<[number, PetPlace[]]>("split_pet", { pet: pet.id, assets: assetIds });
        refresh();
        const n = assetIds.length === 1 ? "1 photo" : `${assetIds.length} photos`;
        toast({ message: `${n} made into a new ${pet.species}`, undo: () => void invoke("restore_pet_places", { places }).then(refresh) });
        return newPet;
      } catch (e) {
        fail(e, "Could not split them.");
        return null;
      }
    },
    [refresh],
  );

  /** `pet` is not in these photos: they leave it, for `to` or for nobody. */
  const movePhotos = useCallback(
    async (pet: PetSummary, assetIds: number[], to: PetSummary | null) => {
      const places = await invoke<PetPlace[]>("move_pet_photos", { pet: pet.id, assets: assetIds, to: to?.id ?? null });
      refresh();
      const n = assetIds.length === 1 ? "1 photo" : `${assetIds.length} photos`;
      toast({
        message: to ? `Moved ${n} to ${petName(to)}` : `Removed ${n} from ${petName(pet)}`,
        undo: () => void invoke("restore_pet_places", { places }).then(refresh),
      });
    },
    [refresh],
  );

  const remove = useCallback(
    async (pet: PetSummary) => {
      await invoke("remove_pet", { pet: pet.id });
      refresh();
      toast({ message: `${pet.name ?? `This ${pet.species}`} removed from Pets`, undo: () => void invoke("restore_removed_pet", { pet: pet.id }).then(refresh) });
    },
    [refresh],
  );

  const restore = useCallback(
    async (pet: PetSummary) => {
      await invoke("restore_removed_pet", { pet: pet.id });
      refresh();
    },
    [refresh],
  );

  /** *Not a pet*: a plush, a statue, a cartoon. */
  const notAPet = useCallback(
    async (pet: PetSummary) => {
      const undo = await invoke<NotAPetUndo>("not_a_pet", { pet: pet.id });
      refresh();
      toast({ message: "Not a pet: those photos are left alone", undo: () => void invoke("undo_not_a_pet", { undo }).then(refresh) });
    },
    [refresh],
  );

  const setCover = useCallback(
    async (pet: number, detection: number) => {
      const before = await invoke<number | null>("set_pet_cover", { pet, detection });
      refresh();
      toast({ message: "Main photo changed", undo: before == null ? undefined : () => void invoke("set_pet_cover", { pet, detection: before }).then(refresh) });
    },
    [refresh],
  );

  return { name, merge, split, movePhotos, remove, restore, notAPet, setCover };
}

/**
 * One pet box at a time (Round 8B): the viewer's *Show pets* boxes and the info panel's pet
 * rows. Each change offers Undo; nothing asks.
 */
export function usePetBoxes() {
  const qc = useQueryClient();
  const refresh = useCallback(() => void Promise.all([qc.invalidateQueries({ queryKey: ["pets"] }), invalidateLayouts(qc)]), [qc]);
  const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });
  const restore = useCallback((places: PetPlace[]) => void invoke("restore_pet_places", { places }).then(refresh), [refresh]);

  /** *Tag as…*: this box is `pet`. */
  const tag = useCallback(
    async (box: AssetPet, pet: PetSummary) => {
      try {
        const place = await invoke<PetPlace>("tag_pet", { detection: box.id, pet: pet.id });
        refresh();
        toast({ message: `Tagged as ${petName(pet)}`, undo: () => restore([place]) });
      } catch (e) {
        fail(e, "Couldn't tag this pet");
      }
    },
    [refresh, restore],
  );

  /** *Not {name}*: an unknown pet again, never offered to that one. */
  const untag = useCallback(
    async (box: AssetPet) => {
      try {
        const place = await invoke<PetPlace>("untag_pet_detection", { detection: box.id });
        refresh();
        toast({ message: `Not ${box.name ?? `this ${box.species}`}`, undo: () => restore([place]) });
      } catch (e) {
        fail(e, "Couldn't change this pet");
      }
    },
    [refresh, restore],
  );

  /** *Not a pet*: this one box only (a toy, a picture of a dog). */
  const reject = useCallback(
    async (box: AssetPet) => {
      try {
        const place = await invoke<PetPlace>("reject_pet_detection", { detection: box.id });
        refresh();
        toast({ message: "Not a pet", undo: () => void invoke("undo_reject_pet_detection", { place }).then(refresh) });
      } catch (e) {
        fail(e, "Couldn't change this pet");
      }
    },
    [refresh],
  );

  /** *New pet…*: a new pet called `name`, holding this box. */
  const create = useCallback(
    async (box: AssetPet, name: string) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      try {
        const [pet, place] = await invoke<[number, PetPlace]>("create_pet_with_detection", { detection: box.id, name: trimmed });
        refresh();
        toast({ message: `New pet: ${trimmed}`, undo: () => void invoke("undo_create_pet", { pet, place }).then(refresh) });
      } catch (e) {
        fail(e, "Couldn't make a new pet");
      }
    },
    [refresh],
  );

  return { tag, untag, reject, create };
}
