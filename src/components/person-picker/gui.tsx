import { Modal, TextInput } from "@mantine/core";
import { IconSearch, IconUser, IconUserPlus } from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** One row: face, name, photo count. The list shows five of them before it scrolls. */
export const ROW_HEIGHT = 64;
const VISIBLE_ROWS = 5;

/** Lower case, accents gone: "Ána" is found by "an". */
const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();

/**
 * *Merge with…* (Round 4B): everyone but `exclude`, named first and then by how often they
 * appear, filtered as you type. Arrow keys move, Enter picks, Escape closes. With `onCreate`
 * (tagging, Round 4I) the last row is *New person…*: what was typed becomes their name.
 * With `group` (Round 9A: pets of the other species) rows sort by group first, and a group
 * with a label starts under that heading.
 */
export function PersonPicker({
  people,
  exclude,
  onPick,
  onCreate,
  onClose,
  title = "Merge with…",
  coverUrl = (id) => mediaUrl("face", id),
  group,
}: {
  people: PersonSummary[];
  exclude?: number;
  onPick: (p: PersonSummary) => void;
  onCreate?: (name: string) => void;
  onClose: () => void;
  title?: string;
  /** The picture of a cover id: a face's crop by default, a pet's for Pets (Round 4C). */
  coverUrl?: (id: number) => string;
  group?: (p: PersonSummary) => { order: number; label: string | null };
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const list = useRef<HTMLUListElement>(null);
  const shown = useMemo(() => {
    const q = fold(query.trim());
    return people
      .filter((p) => p.id !== exclude && (q === "" || (p.name !== null && fold(p.name).includes(q))))
      .sort((a, b) => (group ? group(a).order - group(b).order : 0) || Number(a.name === null) - Number(b.name === null) || (a.name && b.name ? a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) : b.photoCount - a.photoCount));
  }, [people, exclude, query, group]);
  const typed = query.trim();
  // The *New person…* row sits after the people; with nothing typed it asks for a name.
  const createIndex = onCreate ? shown.length : -1;
  const rows = shown.length + (onCreate ? 1 : 0);
  const [askName, setAskName] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const create = () => {
    if (!onCreate) return;
    if (typed) onCreate(typed);
    else {
      setAskName(true);
      input.current?.focus();
    }
  };
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelectorAll('[role="option"]')[active]?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") setActive((i) => Math.min(rows - 1, i + 1));
    else if (e.key === "ArrowUp") setActive((i) => Math.max(0, i - 1));
    else if (e.key === "Enter" && active === createIndex) create();
    else if (e.key === "Enter" && shown[active]) onPick(shown[active]);
    else return;
    e.preventDefault();
  };

  return (
    <Modal opened onClose={onClose} title={title} centered size="sm" classNames={{ content: "person-picker", header: "person-picker__header", body: "person-picker__body" }}>
      <TextInput
        ref={input}
        data-autofocus
        autoFocus
        aria-label="Search people"
        placeholder={askName ? "Type the new person's name" : onCreate ? "Search or type a new name" : "Search people"}
        leftSection={<IconSearch size={16} />}
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={onKeyDown}
      />
      {rows === 0 ? (
        <p className="person-picker__empty">{typed ? `No one called "${typed}"` : "No one else yet."}</p>
      ) : (
        <ul ref={list} className="person-picker__list" role="listbox" aria-label="People" style={{ maxHeight: `${VISIBLE_ROWS * ROW_HEIGHT}px` }}>
          {shown.map((p, i) => {
            const label = group?.(p).label ?? null;
            const heads = label !== null && (i === 0 || group?.(shown[i - 1]).label !== label);
            return [
              heads && (
                <li key={`heading-${label}`} role="presentation" className="person-picker__heading">
                  <h3>{label}</h3>
                </li>
              ),
              <li
                key={p.id}
                role="option"
                aria-selected={i === active}
                className="person-picker__row"
                style={{ height: ROW_HEIGHT }}
                onMouseEnter={() => setActive(i)}
                onClick={() => onPick(p)}
              >
                {p.coverFaceId != null ? (
                  <img className="person-picker__face" src={coverUrl(p.coverFaceId)} alt="" />
                ) : (
                  <span className="person-picker__face">
                    <IconUser size={18} />
                  </span>
                )}
                <span className="person-picker__name" data-unnamed={p.name === null}>
                  {p.name ?? "Unnamed"}
                </span>
                <span className="person-picker__count">
                  {p.photoCount.toLocaleString()} {p.photoCount === 1 ? "photo" : "photos"}
                </span>
              </li>,
            ];
          })}
          {onCreate && (
            <li
              role="option"
              aria-selected={active === createIndex}
              className="person-picker__row person-picker__row--create"
              style={{ height: ROW_HEIGHT }}
              onMouseEnter={() => setActive(createIndex)}
              onClick={create}
            >
              <span className="person-picker__face">
                <IconUserPlus size={18} />
              </span>
              <span className="person-picker__name">{typed ? `New person “${typed}”` : "New person…"}</span>
            </li>
          )}
        </ul>
      )}
    </Modal>
  );
}
