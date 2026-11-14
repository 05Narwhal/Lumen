import { Button, Menu, TextInput } from "@mantine/core";
import { IconArrowLeft, IconArrowsJoin, IconArrowsSplit, IconDots, IconMoodEmpty, IconPawOff, IconPhotoStar, IconUserMinus, IconUserShare } from "@tabler/icons-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import type { MenuItem } from "@/components/context-menu";
import { PersonPicker } from "@/components/person-picker";
import { PhotoGrid } from "@/components/photo-grid";
import { TagBar, TagSearchField } from "@/components/tag-search";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { asPicker, bySpecies, detectionOf, petName, usePetActions, usePets } from "@/hooks/use-pets";
import { clearSelection } from "@/hooks/use-selection";
import { useSettings } from "@/hooks/use-settings";
import { useTagSearch, withTags } from "@/hooks/use-tag-search";
import { ROW_HEIGHTS } from "@/routes/library";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import { mediaUrl } from "@/utils/media";
import { PetSuggestions } from "./suggestions";

/** One pet: its picture, its name (edited in place) and every photo it is in. */
export function PetDetail({ id }: { id: number }) {
  const { pets } = usePets();
  const pet = pets.find((p) => p.id === id);
  const { name, merge, split, movePhotos, remove, notAPet, setCover } = usePetActions();
  const navigate = useNavigate();
  // Photos move only to a pet of the same species; a whole pet merges into any (Round 9A).
  const kin = useMemo(() => pets.filter((p) => p.id !== id && p.species === pet?.species), [pets, id, pet?.species]);
  const others = useMemo(() => pets.filter((p) => p.id !== id), [pets, id]);
  const named = useMemo(() => kin.filter((p) => p.name), [kin]);
  const [merging, setMerging] = useState(false);
  const { settings } = useSettings();
  const [, setParams] = useSearchParams();
  const [draft, setDraft] = useState<string | null>(null);
  // Search tags narrow these photos in place (Round 7).
  const { seq } = useTagSearch();
  const scope = useMemo<LayoutScope>(() => withTags({ ...LIBRARY_SCOPE, petId: id }, seq), [id, seq]);
  const onOpen = useCallback(
    (assetId: number) =>
      setParams((p) => {
        p.set("asset", String(assetId));
        return p;
      }),
    [setParams],
  );

  // Photos grouped here wrongly: another animal (Split), not this one, or a named other one.
  const extraMenuItems = useCallback(
    (ids: number[]): MenuItem[] => {
      if (!pet) return [];
      const n = ids.length === 1 ? "" : ` (${ids.length})`;
      const cover: MenuItem[] =
        ids.length === 1
          ? [
              {
                label: `Use as main photo of ${petName(pet)}`,
                icon: <IconPhotoStar size={16} />,
                onSelect: () =>
                  void detectionOf(pet.id, ids[0]).then((d) => {
                    if (d != null) void setCover(pet.id, d);
                  }),
              },
            ]
          : [];
      return [
        { divider: true },
        ...cover,
        {
          label: `These are a different ${pet.species}${n}`,
          icon: <IconArrowsSplit size={16} />,
          onSelect: () =>
            void split(pet, ids).then((other) => {
              clearSelection();
              if (other != null) navigate(`/pets/${other}`);
            }),
        },
        { label: `Remove from ${petName(pet)}${n}`, icon: <IconUserMinus size={16} />, onSelect: () => void movePhotos(pet, ids, null).then(clearSelection) },
        ...(named.length > 0
          ? [{ label: `Move${n} to`, icon: <IconUserShare size={16} />, children: named.map((o) => ({ label: o.name ?? "", onSelect: () => void movePhotos(pet, ids, o).then(clearSelection) })) }]
          : []),
      ];
    },
    [pet, named, split, movePhotos, setCover, navigate],
  );

  const save = () => {
    if (draft !== null && draft.trim() !== (pet?.name ?? "")) void name(id, draft);
    setDraft(null);
  };

  return (
    <div className="pets-route pets-route--detail">
      <header className="pets-route__hero">
        <Button component={Link} to="/people?show=pets" variant="subtle" size="xs" leftSection={<IconArrowLeft size={14} />}>
          Pets
        </Button>
        {pet?.coverDetectionId != null && <img className="pets-route__face pets-route__face--large" src={mediaUrl("pet", pet.coverDetectionId)} alt="" />}
        <TextInput
          className="pets-route__name-field"
          aria-label="Name"
          placeholder="Add a name"
          value={draft ?? pet?.name ?? ""}
          onChange={(e) => setDraft(e.currentTarget.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setDraft(null);
          }}
        />
        {pet && others.length > 0 && (
          <Button variant="subtle" size="xs" leftSection={<IconArrowsJoin size={14} />} onClick={() => setMerging(true)}>
            Merge with…
          </Button>
        )}
        {pet && merging && (
          <PersonPicker
            people={others.map(asPicker)}
            group={bySpecies(pets, pet.species)}
            exclude={pet.id}
            coverUrl={(d) => mediaUrl("pet", d)}
            onClose={() => setMerging(false)}
            onPick={(o) => {
              setMerging(false);
              const into = others.find((k) => k.id === o.id);
              if (into) void merge(into, pet).then(() => navigate(`/pets/${into.id}`));
            }}
          />
        )}
        {pet && (
          <Menu position="bottom-end" withinPortal>
            <Menu.Target>
              <Button variant="subtle" size="xs" aria-label="More" leftSection={<IconDots size={14} />}>
                More
              </Button>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item leftSection={<IconMoodEmpty size={14} />} onClick={() => void notAPet(pet).then(() => navigate("/people?show=pets"))}>
                Not a pet
              </Menu.Item>
              <Menu.Item leftSection={<IconPawOff size={14} />} onClick={() => void remove(pet).then(() => navigate("/people?show=pets"))}>
                Remove from Pets
              </Menu.Item>
            </Menu.Dropdown>
          </Menu>
        )}
        <div className="pets-route__search">
          <TagSearchField />
        </div>
      </header>
      {pet && <PetSuggestions pet={pet} />}
      <TagBar />
      <div className="pets-route__grid-host">
        <PhotoGrid rowHeight={ROW_HEIGHTS[settings?.appearance.tileSize ?? "m"]} scope={scope} onOpen={onOpen} extraMenuItems={extraMenuItems} />
      </div>
    </div>
  );
}
