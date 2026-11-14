import { Button, Menu, TextInput } from "@mantine/core";
import {
  IconArrowLeft,
  IconCheck,
  IconDots,
  IconExternalLink,
  IconFolderPlus,
  IconMoodEmpty,
  IconPencil,
  IconPlus,
  IconTag,
  IconTrash,
  IconUserOff,
  IconUsers,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { type MenuItem, useContextMenu } from "@/components/context-menu";
import { EmptyState } from "@/components/empty-state";
import { PersonPicker } from "@/components/person-picker";
import { usePeople, usePeopleActions, usePersonGroups } from "@/hooks/use-people";
import { personMenu as personMenuFor } from "@/lib/menus";
import type { PersonGroup } from "@/types/generated/PersonGroup";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import { mediaUrl } from "@/utils/media";
import { usePersonDrag } from "./use-person-drag";


/** How many faces a group's circle shows. */
const GROUP_FACES = 3;

const plural = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/**
 * Everyone the face models found (Phase 7b). The user's groups come first, each a circle of
 * up to three of its members' faces, then everyone in no group. Dropping a person on
 * another merges them (with Undo); on a group, adds them to it; on *New group*, starts one.
 * `?group=<id>` shows one group's members. `compact` (the *All* view of People & Pets) shrinks the
 * empty state to one line.
 */
export function PeopleBody({ compact = false }: { compact?: boolean }) {
  const { people, isLoading: peopleLoading, isError: peopleError, retry: retryPeople } = usePeople();
  const { name, merge, remove, reject } = usePeopleActions();
  const groupsApi = usePersonGroups();
  const { groups } = groupsApi;
  // The grid waits for both lists: drawn without its groups, everyone shows ungrouped.
  const isLoading = peopleLoading || groupsApi.isLoading;
  const loadFailed = peopleError || groupsApi.isError;
  const [params, setParams] = useSearchParams();
  const [renaming, setRenaming] = useState<number | null>(null);
  const [merging, setMerging] = useState<PersonSummary | null>(null);
  // Naming a new group (optionally started by dropping a person on *New group*).
  const [creating, setCreating] = useState<{ people?: number[] } | null>(null);
  // Select mode (started by a person's *Select*): a click toggles instead of opening.
  const [selected, setSelected] = useState<Set<number> | null>(null);
  const selecting = selected !== null;
  const endSelect = () => {
    setSelected(null);
    anchor.current = null;
  };
  // The last person clicked, where a Shift-click's range starts.
  const anchor = useRef<number | null>(null);
  /** A click while selecting, or a Ctrl/Cmd/Shift-click at any time (which starts selecting). */
  const clickSelect = (id: number, e: React.MouseEvent, order: PersonSummary[]) => {
    const from = anchor.current != null ? order.findIndex((p) => p.id === anchor.current) : -1;
    const to = order.findIndex((p) => p.id === id);
    if (e.shiftKey && from >= 0 && to >= 0) {
      const [a, b] = from < to ? [from, to] : [to, from];
      const range = order.slice(a, b + 1).map((p) => p.id);
      // A range adds to what is already chosen, as in the photo grid.
      setSelected((s) => new Set([...(s ?? []), ...range]));
      return;
    }
    anchor.current = id;
    setSelected((s) => {
      const next = new Set(s ?? []);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  };
  useEffect(() => {
    if (!selecting) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && endSelect();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selecting]);
  const [renamingGroup, setRenamingGroup] = useState(false);
  const { open } = useContextMenu();
  const navigate = useNavigate();

  const byId = new Map(people.map((p) => [p.id, p]));
  const openGroup = groups.find((g) => String(g.id) === params.get("group")) ?? null;
  const membersOf = (g: PersonGroup) =>
    g.personIds
      .map((id) => byId.get(id))
      .filter((p): p is PersonSummary => !!p)
      .sort((a, b) => b.photoCount - a.photoCount);
  const grouped = new Set(groups.flatMap((g) => g.personIds));
  const loose = people.filter((p) => !grouped.has(p.id));

  // Where a dropped person goes: onto a person merges, onto a group joins it, onto *New
  // group* starts one with them.
  // Dragging a selected person carries the whole selection into a group.
  const { drag, handle, ghostRef } = usePersonDrag((id, target) => {
    const person = byId.get(id);
    if (!person) return;
    const ids = selected?.has(id) ? [...selected] : [id];
    const [kind, key] = target.split(":");
    if (kind === "person") {
      const into = byId.get(Number(key));
      if (into && into.id !== person.id && ids.length === 1) void merge(into, person);
      return;
    }
    if (kind === "group") void groupsApi.setMember(Number(key), ids, true);
    else if (kind === "new") setCreating({ people: ids });
    endSelect();
  });
  const dropZone = (key: string) => ({ "data-drop": key, "data-over": drag.over === key });

  const createGroup = async (value: string) => {
    const ids = creating?.people ?? [];
    setCreating(null);
    if (value.trim()) await groupsApi.create(value, ids);
  };

  const personMenu = (p: PersonSummary): MenuItem[] =>
    personMenuFor(p, {
      open: () => navigate(`/people/${p.id}`),
      select: () => {
        anchor.current = p.id;
        setSelected(new Set([...(selected ?? []), p.id]));
      },
      rename: () => setRenaming(p.id),
      merge: () => setMerging(p),
      canMerge: people.length >= 2,
      groups: [
        {
          label: "Groups",
          icon: <IconTag size={14} />,
          children: [
            ...groups.map((g) => {
              const on = g.personIds.includes(p.id);
              return {
                label: g.name,
                icon: on ? <IconCheck size={14} /> : <span className="people-route__menu-spacer" />,
                onSelect: () => void groupsApi.setMember(g.id, [p.id], !on),
              };
            }),
            ...(groups.length ? [{ divider: true as const }] : []),
            { label: "New group…", icon: <IconFolderPlus size={14} />, onSelect: () => setCreating({ people: [p.id] }) },
          ],
        },
        ...(openGroup
          ? [{ label: `Remove from ${openGroup.name}`, icon: <IconTag size={14} />, onSelect: () => void groupsApi.setMember(openGroup.id, [p.id], false) }]
          : []),
      ],
      notAPerson: () => void reject(p),
      remove: () => void remove(p),
    });

  const groupMenu = (g: PersonGroup): MenuItem[] => [
    { label: "Open", icon: <IconExternalLink size={14} />, onSelect: () => setParams({ group: String(g.id) }) },
    { label: "Rename group", icon: <IconPencil size={14} />, onSelect: () => (setParams({ group: String(g.id) }), setRenamingGroup(true)) },
    { divider: true },
    { label: "Delete group", icon: <IconTrash size={14} />, danger: true, onSelect: () => void groupsApi.remove(g) },
  ];

  // What a Shift-click's range runs over: the people on screen, in their order.
  const order = openGroup ? membersOf(openGroup) : loose;

  const personCard = (p: PersonSummary) => (
    <li
      key={p.id}
      className="people-route__card"
      data-dragging={drag.person === p.id || (drag.person != null && !!selected?.has(drag.person) && selected.has(p.id))}
      data-selected={selected?.has(p.id) ?? false}
      {...handle(p.id)}
      onContextMenu={(e) => open(e, personMenu(p))}
      {...dropZone(`person:${p.id}`)}
    >
      <Link
        to={`/people/${p.id}`}
        className="people-route__link"
        draggable={false}
        aria-pressed={selected ? selected.has(p.id) : undefined}
        onClick={(e) => {
          if (!selecting && !e.shiftKey && !e.ctrlKey && !e.metaKey) return;
          e.preventDefault();
          clickSelect(p.id, e, order);
        }}
      >
        {selecting && (
          <span className="people-route__check" aria-hidden>
            <IconCheck size={14} stroke={3} />
          </span>
        )}
        {p.coverFaceId != null ? (
          <img className="people-route__face" src={mediaUrl("face", p.coverFaceId)} alt="" loading="lazy" draggable={false} />
        ) : (
          <span className="people-route__face" aria-hidden />
        )}
        {renaming === p.id ? (
          <NameField
            label="Name"
            initial={p.name ?? ""}
            onDone={(v) => {
              setRenaming(null);
              if (v !== null && v !== (p.name ?? "")) void name(p.id, v);
            }}
          />
        ) : (
          <span className="people-route__name">{p.name ?? "Add a name"}</span>
        )}
        <span className="people-route__count">{plural(p.photoCount, "photo", "photos")}</span>
      </Link>
    </li>
  );

  const dragged = drag.person != null ? byId.get(drag.person) : undefined;
  const ghost = dragged && (
    <div ref={(el) => void (ghostRef.current = el)} className="people-route__ghost" aria-hidden>
      {dragged.coverFaceId != null && <img src={mediaUrl("face", dragged.coverFaceId)} alt="" draggable={false} />}
      {selected?.has(dragged.id) && selected.size > 1 && <span className="people-route__ghost-count">{selected.size}</span>}
    </div>
  );

  const chosen = selected ? people.filter((p) => selected.has(p.id)) : [];
  const bar = selected && (
    <div className="people-route__selbar" role="toolbar" aria-label="Selection">
      <span className="people-route__selbar-count">{chosen.length ? `${plural(chosen.length, "person", "people")} selected` : "Click people to select them"}</span>
      <Button variant="subtle" size="xs" onClick={() => setSelected(new Set((openGroup ? membersOf(openGroup) : people).map((p) => p.id)))}>
        Select all
      </Button>
      <Menu position="top" withinPortal>
        <Menu.Target>
          <Button variant="default" size="xs" leftSection={<IconTag size={14} />} disabled={!chosen.length}>
            Add to group
          </Button>
        </Menu.Target>
        <Menu.Dropdown>
          {groups.map((g) => (
            <Menu.Item key={g.id} onClick={() => void groupsApi.setMember(g.id, [...selected], true).then(endSelect)}>
              {g.name}
            </Menu.Item>
          ))}
          {groups.length > 0 && <Menu.Divider />}
          <Menu.Item
            leftSection={<IconFolderPlus size={14} />}
            onClick={() => {
              setCreating({ people: [...selected] });
              endSelect();
              setParams({});
            }}
          >
            New group…
          </Menu.Item>
        </Menu.Dropdown>
      </Menu>
      {openGroup && (
        <Button variant="default" size="xs" disabled={!chosen.length} onClick={() => void groupsApi.setMember(openGroup.id, [...selected], false).then(endSelect)}>
          Remove from {openGroup.name}
        </Button>
      )}
      <Button variant="default" size="xs" leftSection={<IconMoodEmpty size={14} />} disabled={!chosen.length} onClick={() => void reject(chosen).then(endSelect)}>
        Not a person
      </Button>
      <Button variant="default" size="xs" color="red" leftSection={<IconUserOff size={14} />} disabled={!chosen.length} onClick={() => void remove(chosen).then(endSelect)}>
        Remove from People
      </Button>
      <Button size="xs" onClick={endSelect}>
        Done
      </Button>
    </div>
  );

  const picker = merging && (
    <PersonPicker
      people={people}
      exclude={merging.id}
      onClose={() => setMerging(null)}
      onPick={(into) => {
        const from = merging;
        setMerging(null);
        void merge(into, from);
      }}
    />
  );

  if (openGroup) {
    const members = membersOf(openGroup);
    return (
      <div className="people-route">
        <header className="people-route__group-head">
          <Link to="/people" className="people-route__back">
            <IconArrowLeft size={16} /> People
          </Link>
          <GroupFaces members={members} size="small" />
          {renamingGroup ? (
            <NameField
              label="Group name"
              initial={openGroup.name}
              onDone={(v) => {
                setRenamingGroup(false);
                if (v && v !== openGroup.name) void groupsApi.rename(openGroup.id, v);
              }}
            />
          ) : (
            <button type="button" className="people-route__group-title" onClick={() => setRenamingGroup(true)} title="Rename">
              {openGroup.name}
            </button>
          )}
          <span className="people-route__count">{plural(members.length, "person", "people")}</span>
          <Menu position="bottom-end" withinPortal>
            <Menu.Target>
              <Button variant="subtle" size="xs" aria-label="More" leftSection={<IconDots size={14} />}>
                More
              </Button>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item leftSection={<IconPencil size={14} />} onClick={() => setRenamingGroup(true)}>
                Rename group
              </Menu.Item>
              <Menu.Item
                color="red"
                leftSection={<IconTrash size={14} />}
                onClick={() => {
                  setParams({});
                  void groupsApi.remove(openGroup);
                }}
              >
                Delete group
              </Menu.Item>
            </Menu.Dropdown>
          </Menu>
        </header>
        {members.length === 0 ? (
          <EmptyState
            icon={<IconUsers />}
            title="Nobody here yet"
            description="Drag people onto this group on the People page, or right-click someone and choose Groups."
          />
        ) : (
          <ul className="people-route__grid" aria-label={`People in ${openGroup.name}`}>
            {members.map(personCard)}
          </ul>
        )}
        {picker}
        {ghost}
        {bar}
      </div>
    );
  }

  return (
    <div className="people-route">
      {loadFailed ? (
        <EmptyState
          icon={<IconUsers />}
          title="People could not load."
          description="Lumen is still busy or the library could not be read."
          action={
            <Button
              onClick={() => {
                if (peopleError) retryPeople();
                if (groupsApi.isError) groupsApi.retry();
              }}
            >
              Try again
            </Button>
          }
        />
      ) : isLoading ? null : people.length === 0 ? (
        compact ? (
          <p className="people-route__none">No people found yet.</p>
        ) : (
          <EmptyState
            icon={<IconUsers />}
            title="No people yet"
            description="People appear once Lumen has looked for faces in your photos."
          />
        )
      ) : (
        <ul className="people-route__grid" aria-label="People">
          {groups.map((g) => {
            const members = membersOf(g);
            return (
              <li
                key={`g${g.id}`}
                className="people-route__card people-route__card--group"
                onContextMenu={(e) => open(e, groupMenu(g))}
                {...dropZone(`group:${g.id}`)}
              >
                <Link to={`/people?group=${g.id}`} className="people-route__link" draggable={false}>
                  <GroupFaces members={members} />
                  <span className="people-route__name">{g.name}</span>
                  <span className="people-route__count">{plural(members.length, "person", "people")}</span>
                </Link>
              </li>
            );
          })}
          <li className="people-route__card people-route__card--new" {...dropZone("new")}>
            {creating ? (
              <div className="people-route__link">
                <span className="people-route__face people-route__face--new" aria-hidden>
                  {creating.people?.length && byId.get(creating.people[0])?.coverFaceId != null ? (
                    <img src={mediaUrl("face", byId.get(creating.people[0])!.coverFaceId!)} alt="" draggable={false} />
                  ) : (
                    <IconPlus size={28} />
                  )}
                </span>
                <NameField label="Group name" initial="" placeholder="Family, Friends…" onDone={(v) => void createGroup(v ?? "")} />
              </div>
            ) : (
              <button type="button" className="people-route__link" onClick={() => setCreating({})}>
                <span className="people-route__face people-route__face--new" aria-hidden>
                  <IconPlus size={28} />
                </span>
                <span className="people-route__name">New group</span>
                <span className="people-route__count">or drop people here</span>
              </button>
            )}
          </li>
          {loose.map(personCard)}
        </ul>
      )}
      {picker}
      {ghost}
      {bar}
    </div>
  );
}

/** Up to three members' faces clustered inside one circle, largest first. */
function GroupFaces({ members, size }: { members: PersonSummary[]; size?: "small" }) {
  const faces = members.filter((m) => m.coverFaceId != null).slice(0, GROUP_FACES);
  return (
    <span className="people-route__stack" data-count={faces.length} data-size={size} aria-hidden>
      {faces.length === 0 ? (
        <IconUsers size={size ? 18 : 32} />
      ) : (
        faces.map((m) => <img key={m.id} className="people-route__stack-face" src={mediaUrl("face", m.coverFaceId!)} alt="" loading="lazy" draggable={false} />)
      )}
    </span>
  );
}

/** An inline name field: Enter or blur commits the trimmed value, Escape gives `null`. */
function NameField({ label, initial, placeholder, onDone }: { label: string; initial: string; placeholder?: string; onDone: (v: string | null) => void }) {
  const [cancelled, setCancelled] = useState(false);
  return (
    <TextInput
      size="xs"
      autoFocus
      aria-label={label}
      placeholder={placeholder}
      className="people-route__name-input"
      defaultValue={initial}
      onClick={(e) => e.preventDefault()}
      onBlur={(e) => onDone(cancelled ? null : e.currentTarget.value.trim())}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setCancelled(true);
          requestAnimationFrame(() => (e.target as HTMLInputElement).blur());
        }
      }}
    />
  );
}
