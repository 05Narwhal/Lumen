import { useState, type ReactNode } from "react";
import { useNavigate } from "react-router";
import { promptName } from "@/components/confirm-dialog";
import type { MenuItem } from "@/components/context-menu";
import { PersonPicker } from "@/components/person-picker";
import { asPicker, usePetActions, usePetBoxes, usePets } from "@/hooks/use-pets";
import { petBoxMenu } from "@/lib/menus";
import type { AssetPet } from "@/types/generated/AssetPet";
import { mediaUrl } from "@/utils/media";

/**
 * A pet box's menu (Round 8B), for the viewer's *Show pets* boxes and the info panel's pet rows:
 * `menuFor(box)` builds it, and `picker` is the *Tag as…* chooser to render beside it.
 * `withOpen` adds *Open {name}* (the info panel's rows).
 */
export function usePetBoxMenu(): { menuFor: (box: AssetPet, withOpen?: boolean) => MenuItem[]; tagBox: (box: AssetPet) => void; picker: ReactNode } {
  const { pets } = usePets();
  const boxes = usePetBoxes();
  const { setCover } = usePetActions();
  const navigate = useNavigate();
  const [tagging, setTagging] = useState<AssetPet | null>(null);
  // A cat is only ever tagged as a cat, and not as the pet it already is.
  const kin = tagging ? pets.filter((p) => p.species === tagging.species && p.id !== tagging.petId) : [];

  const menuFor = (box: AssetPet, withOpen = false) =>
    petBoxMenu(box, {
      open: withOpen && box.petId != null ? () => navigate(`/pets/${box.petId}`) : undefined,
      tag: () => setTagging(box),
      newPet: () =>
        void promptName({ title: `New ${box.species}`, initial: "", confirmLabel: "Create" }).then((name) => {
          if (name != null) void boxes.create(box, name);
        }),
      useAsMain: box.petId != null ? () => void setCover(box.petId!, box.id) : undefined,
      untag: box.petId != null ? () => void boxes.untag(box) : undefined,
      notAPet: () => void boxes.reject(box),
    });

  const picker = tagging ? (
    <PersonPicker
      title={`Which ${tagging.species} is this?`}
      people={kin.map(asPicker)}
      coverUrl={(d) => mediaUrl("pet", d)}
      onClose={() => setTagging(null)}
      onPick={(p) => {
        const box = tagging;
        setTagging(null);
        const pet = pets.find((k) => k.id === p.id);
        if (pet) void boxes.tag(box, pet);
      }}
    />
  ) : null;

  return { menuFor, tagBox: setTagging, picker };
}
