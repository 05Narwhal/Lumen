import { IconCat, IconDog, IconPaw } from "@tabler/icons-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { useContextMenu } from "@/components/context-menu";
import { promptName } from "@/components/confirm-dialog";
import { EmptyState } from "@/components/empty-state";
import { PersonPicker } from "@/components/person-picker";
import { Spinner } from "@/components/spinner";
import { asPicker, bySpecies, usePetActions, usePets } from "@/hooks/use-pets";
import { petMenu } from "@/lib/menus";
import type { PetSummary } from "@/types/generated/PetSummary";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

const plural = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

function PetCard({ pet, onMenu }: { pet: PetSummary; onMenu: (e: React.MouseEvent) => void }) {
  const Icon = pet.species === "cat" ? IconCat : IconDog;
  return (
    <li className="pets-route__card" onContextMenu={onMenu}>
      <Link to={`/pets/${pet.id}`} className="pets-route__link" aria-label={pet.name ?? `Unnamed ${pet.species}`}>
        {pet.coverDetectionId != null ? (
          <img className="pets-route__face" src={mediaUrl("pet", pet.coverDetectionId)} alt="" loading="lazy" draggable={false} />
        ) : (
          <span className="pets-route__face" aria-hidden />
        )}
        <span className="pets-route__name">
          <Icon size={14} aria-hidden className="pets-route__species" />
          {pet.name ?? "Add a name"}
        </span>
        <span className="pets-route__count">{plural(pet.photoCount, "photo", "photos")}</span>
      </Link>
    </li>
  );
}

/**
 * Every pet, named first (Round 4C), each with its menu (Round 8B). People's select mode is
 * built around its groups bar, so pets have no *Select* until they have something to act on
 * together. `compact` (the *All* view of People & Pets) shrinks the empty state to one line.
 */
export function PetsBody({ compact = false }: { compact?: boolean }) {
  const { pets, isLoading } = usePets();
  const { name, merge, notAPet, remove } = usePetActions();
  const { open } = useContextMenu();
  const navigate = useNavigate();
  const [merging, setMerging] = useState<PetSummary | null>(null);
  // Every other pet, the other species too (Round 9A).
  const kin = (p: PetSummary) => pets.filter((o) => o.id !== p.id);

  const menuFor = (p: PetSummary) =>
    petMenu(p, {
      open: () => navigate(`/pets/${p.id}`),
      rename: () =>
        void promptName({ title: p.name ? `Rename ${p.name}` : `Name this ${p.species}`, initial: p.name ?? "", confirmLabel: p.name ? "Rename" : "Save" }).then((n) => {
          if (n != null) void name(p.id, n);
        }),
      merge: () => setMerging(p),
      canMerge: kin(p).length > 0,
      notAPet: () => void notAPet(p),
      remove: () => void remove(p),
    });

  return (
    <div className="pets-route">
      {isLoading ? (
        <div className="pets-route__loading">
          <Spinner size="md" label="Loading pets" />
        </div>
      ) : pets.length === 0 ? (
        compact ? (
          <p className="pets-route__none">No pets found yet.</p>
        ) : (
          <EmptyState icon={<IconPaw />} title="No pets yet" description="Pets appear here once smart features have looked at your photos." />
        )
      ) : (
        <ul className="pets-route__grid">
          {pets.map((p) => (
            <PetCard key={p.id} pet={p} onMenu={(e) => open(e, menuFor(p))} />
          ))}
        </ul>
      )}
      {merging && (
        <PersonPicker
          title={`Merge ${merging.name ?? `this ${merging.species}`} with…`}
          people={kin(merging).map(asPicker)}
          group={bySpecies(pets, merging.species)}
          exclude={merging.id}
          coverUrl={(d) => mediaUrl("pet", d)}
          onClose={() => setMerging(null)}
          onPick={(o) => {
            const from = merging;
            setMerging(null);
            const into = pets.find((k) => k.id === o.id);
            if (into) void merge(into, from);
          }}
        />
      )}
    </div>
  );
}
