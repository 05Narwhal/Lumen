import { Button, Menu, TextInput } from "@mantine/core";
import { IconArrowLeft, IconArrowsJoin, IconDots, IconMoodEmpty, IconPhotoStar, IconUserMinus, IconUserOff, IconUserShare } from "@tabler/icons-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import type { MenuItem } from "@/components/context-menu";
import { PersonPicker } from "@/components/person-picker";
import { PhotoGrid } from "@/components/photo-grid";
import { TagBar, TagSearchField } from "@/components/tag-search";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { faceOf, usePeople, usePeopleActions } from "@/hooks/use-people";
import { clearSelection } from "@/hooks/use-selection";
import { useSettings } from "@/hooks/use-settings";
import { useTagSearch, withTags } from "@/hooks/use-tag-search";
import { ROW_HEIGHTS } from "@/routes/library";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import { mediaUrl } from "@/utils/media";
import { Suggestions } from "./suggestions";

/** One person: their face, their name (edited in place) and every photo they are in. */
export function PersonDetail({ id }: { id: number }) {
  const { people } = usePeople();
  const person = people.find((p) => p.id === id);
  const { name, merge, movePhotos, remove, reject, setCover } = usePeopleActions();
  const navigate = useNavigate();
  // Named people only for *Move to*, as on the People page; nobody unnamed is offered by number.
  const others = useMemo(() => people.filter((p) => p.id !== id && p.name), [people, id]);
  const [merging, setMerging] = useState(false);
  const { settings } = useSettings();
  const [, setParams] = useSearchParams();
  const [draft, setDraft] = useState<string | null>(null);
  // Search tags narrow these photos in place (Round 7).
  const { seq } = useTagSearch();
  const scope = useMemo<LayoutScope>(() => withTags({ ...LIBRARY_SCOPE, personId: id }, seq), [id, seq]);
  const onOpen = useCallback(
    (assetId: number) =>
      setParams((p) => {
        p.set("asset", String(assetId));
        return p;
      }),
    [setParams],
  );

  // Faces the models put here wrongly: out of this person, or over to the right one.
  const extraMenuItems = useCallback(
    (ids: number[]): MenuItem[] => {
      if (!person) return [];
      const n = ids.length === 1 ? "" : ` ${ids.length} photos`;
      const cover: MenuItem[] =
        ids.length === 1
          ? [
              {
                label: "Use as main photo",
                icon: <IconPhotoStar size={16} />,
                onSelect: () =>
                  void faceOf(person.id, ids[0]).then((face) => {
                    if (face != null) void setCover(person.id, face);
                  }),
              },
            ]
          : [];
      return [
        { divider: true },
        ...cover,
        {
          label: `Not ${person.name ?? "this person"}${n ? ` (${ids.length})` : ""}`,
          icon: <IconUserMinus size={16} />,
          onSelect: () => void movePhotos(person, ids, null).then(clearSelection),
        },
        ...(others.length > 0
          ? [
              {
                label: `Move${n} to`,
                icon: <IconUserShare size={16} />,
                children: others.map((o) => ({ label: o.name ?? "", onSelect: () => void movePhotos(person, ids, o).then(clearSelection) })),
              },
            ]
          : []),
      ];
    },
    [person, others, movePhotos, setCover],
  );

  const save = () => {
    if (draft !== null && draft.trim() !== (person?.name ?? "")) void name(id, draft);
    setDraft(null);
  };

  return (
    <div className="people-route people-route--detail">
      <header className="people-route__hero">
        <Button component={Link} to="/people" variant="subtle" size="xs" leftSection={<IconArrowLeft size={14} />}>
          People
        </Button>
        {person?.coverFaceId != null && <img className="people-route__face people-route__face--large" src={mediaUrl("face", person.coverFaceId)} alt="" />}
        <TextInput
          className="people-route__name-field"
          aria-label="Name"
          placeholder="Add a name"
          value={draft ?? person?.name ?? ""}
          onChange={(e) => setDraft(e.currentTarget.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setDraft(null);
          }}
        />
        {person && people.length > 1 && (
          <Button variant="subtle" size="xs" leftSection={<IconArrowsJoin size={14} />} onClick={() => setMerging(true)}>
            Merge with…
          </Button>
        )}
        {person && merging && (
          <PersonPicker
            people={people}
            exclude={person.id}
            onClose={() => setMerging(false)}
            onPick={(o) => {
              setMerging(false);
              void merge(o, person).then(() => navigate(`/people/${o.id}`));
            }}
          />
        )}
        {person && (
          <Menu position="bottom-end" withinPortal>
            <Menu.Target>
              <Button variant="subtle" size="xs" aria-label="More" leftSection={<IconDots size={14} />}>
                More
              </Button>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item leftSection={<IconMoodEmpty size={14} />} onClick={() => void reject(person).then(() => navigate("/people"))}>
                Not a person
              </Menu.Item>
              <Menu.Item leftSection={<IconUserOff size={14} />} onClick={() => void remove(person).then(() => navigate("/people"))}>
                Remove from People
              </Menu.Item>
            </Menu.Dropdown>
          </Menu>
        )}
        <div className="people-route__search">
          <TagSearchField />
        </div>
      </header>
      {person && <Suggestions person={person} />}
      <TagBar />
      <div className="people-route__grid-host">
        <PhotoGrid rowHeight={ROW_HEIGHTS[settings?.appearance.tileSize ?? "m"]} scope={scope} onOpen={onOpen} extraMenuItems={extraMenuItems} />
      </div>
    </div>
  );
}
