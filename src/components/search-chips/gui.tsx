import { Menu } from "@mantine/core";
import { IconBrackets, IconCalendar, IconCamera, IconChevronDown, IconFolders, IconMapPin, IconPaw, IconPhoto, IconPhotoStar, IconTag, IconUser, IconX } from "@tabler/icons-react";
import { PathName } from "@/components/album-name";
import type { Chip } from "@/types/generated/Chip";
import type { ChipKind } from "@/types/generated/ChipKind";
import "./styles.scss";

const ICONS: Record<ChipKind, typeof IconUser> = {
  person: IconUser,
  pet: IconPaw,
  place: IconMapPin,
  album: IconPhotoStar,
  collection: IconFolders,
  date: IconCalendar,
  camera: IconCamera,
  tag: IconTag,
  kind: IconPhoto,
  op: IconBrackets,
};

/** How a reading is named in the alternatives menu: "Florence — the person". */
const READING: Record<ChipKind, string> = {
  person: "the person",
  pet: "the pet",
  place: "the place",
  album: "the album",
  collection: "the collection",
  date: "a date",
  camera: "the camera",
  tag: "what the photo shows",
  kind: "a kind of photo",
  op: "a joining word",
};

/** A chip's words; an album or collection inside collections says where, so two *Italy* albums differ. */
function ChipName({ chip }: { chip: Chip }) {
  return chip.path.length > 0 ? <PathName name={chip.label} path={chip.path} /> : <>{chip.label}</>;
}

/** The plain-text name for labels: `Italy (Trips › Europe)`. */
const plainName = (c: Chip) => (c.path.length ? `${c.label} (${c.path.join(" › ")})` : c.label);

function KindIcon({ kind, className }: { kind: ChipKind; className?: string }) {
  const Icon = ICONS[kind];
  return <Icon size={14} aria-hidden className={className} />;
}

/**
 * What a search's text was read as (Round 4K): one chip per recognised part, each removable,
 * and an ambiguous one with a menu of its other readings. The words nothing recognised follow
 * as plain text.
 */
export function SearchChips({
  chips,
  rest,
  onRemove,
  onChoose,
}: {
  chips: Chip[];
  rest: string;
  onRemove: (chip: Chip) => void;
  /** Read `chip`'s words as `reading` instead. */
  onChoose: (chip: Chip, reading: Chip) => void;
}) {
  if (chips.length === 0 && !rest.trim()) return null;
  return (
    <div className="search-chips">
      <ul className="search-chips__list" aria-label="Search terms">
        {chips.map((c) => {
          // `or`, `and`, `not` and brackets join the term chips: shown, never edited on their own.
          if (c.kind === "op") {
            return (
              <li key={`${c.span[0]}-op-${c.label}`} className="search-chips__op" data-kind="op">
                {c.label}
              </li>
            );
          }
          return (
            <li key={`${c.span[0]}-${c.kind}-${c.label}`} className="search-chips__chip" data-kind={c.kind}>
              <KindIcon kind={c.kind} className="search-chips__icon" />
              <ChipName chip={c} />
              {c.alternatives.length > 0 && (
                <Menu position="bottom-start" withinPortal>
                  <Menu.Target>
                    <button type="button" className="search-chips__button" aria-label={`${plainName(c)}: other meanings`} title="Other meanings">
                      <IconChevronDown size={12} aria-hidden />
                    </button>
                  </Menu.Target>
                  <Menu.Dropdown>
                    <Menu.Label>{`${plainName(c)} — ${READING[c.kind]}`}</Menu.Label>
                    {c.alternatives.map((alt) => (
                      <Menu.Item key={alt.token} leftSection={<KindIcon kind={alt.kind} />} onClick={() => onChoose(c, alt)}>
                        {`${plainName(alt)} — ${READING[alt.kind]}`}
                      </Menu.Item>
                    ))}
                  </Menu.Dropdown>
                </Menu>
              )}
              <button type="button" className="search-chips__button" aria-label={`Remove ${plainName(c)}`} title="Remove" onClick={() => onRemove(c)}>
                <IconX size={12} aria-hidden />
              </button>
            </li>
          );
        })}
      </ul>
      {rest.trim() && (
        <span className="search-chips__rest">
          {chips.length ? "and " : "Photos showing "}
          <q>{rest}</q>
        </span>
      )}
    </div>
  );
}
