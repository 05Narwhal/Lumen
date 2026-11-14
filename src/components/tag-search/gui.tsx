import {
  IconAperture,
  IconPalette,
  IconVolume,
  IconVolumeOff,
  IconAppWindow,
  IconAdjustments,
  IconCalendar,
  IconCalendarTime,
  IconCamera,
  IconClockHour4,
  IconDatabase,
  IconDimensions,
  IconFile,
  IconFileExport,
  IconFileTime,
  IconFlag,
  IconHdr,
  IconFolders,
  IconLibraryPhoto,
  IconLivePhoto,
  IconMapPin,
  IconMapPinOff,
  IconMountain,
  IconMovie,
  IconPanoramaHorizontal,
  IconPaw,
  IconPhoto,
  IconRectangle,
  IconRectangleVertical,
  IconScreenshot,
  IconSearch,
  IconSparkles,
  IconSquare,
  IconStar,
  IconStarFilled,
  IconTextRecognition,
  IconUser,
  IconUserCircle,
  IconVideo,
  IconWand,
  IconX,
} from "@tabler/icons-react";
import { Popover } from "@mantine/core";
import { AnimatePresence, motion } from "motion/react";
import { Fragment, useEffect, useId, useMemo, useRef, useState } from "react";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { useSettings } from "@/hooks/use-settings";
import { type TagStore, useSentenceReading, useTagSearch, useTagSuggestions } from "@/hooks/use-tag-search";
import { albumPath, collectionPath } from "@/lib/album-collections";
import { describe, replaceLastTag } from "@/lib/search-expr";
import { highlight, sameTag, tagKey } from "@/lib/search-tags";
import type { SentenceReading } from "@/types/generated/SentenceReading";
import type { TagSuggestion } from "@/types/generated/TagSuggestion";
import type { TagValue } from "@/types/generated/TagValue";
import { TokenRow } from "./token-row";
import "./styles.scss";

/** Tags arrive and leave on the same spring as the sidebar's pill. */
const SPRING = {
  type: "spring",
  stiffness: 520,
  damping: 38,
  mass: 0.8,
} as const;

/** The icon that says what kind of thing a tag is. */
export function TagIcon({ value, size = 14 }: { value: TagValue; size?: number }) {
  const props = { size, "aria-hidden": true, className: "tag-icon" } as const;
  switch (value.type) {
    case "person":
      return <IconUser {...props} />;
    case "pet":
      return <IconPaw {...props} />;
    case "place":
      return <IconMapPin {...props} />;
    case "location":
      return value.has ? <IconMapPin {...props} /> : <IconMapPinOff {...props} />;
    case "album":
      return <IconLibraryPhoto {...props} />;
    case "collection":
      return <IconFolders {...props} />;
    case "scene":
      return <IconSparkles {...props} />;
    case "dates":
      switch (value.field) {
        case "imported":
          return <IconCalendarTime {...props} />;
        case "fileModified":
        case "fileCreated":
          return <IconFileTime {...props} />;
        case "digitized":
          return <IconCalendarTime {...props} />;
        default:
          return <IconCalendar {...props} />;
      }
    case "months":
      return <IconCalendar {...props} />;
    case "media":
      return value.media === 1 ? <IconVideo {...props} /> : <IconPhoto {...props} />;
    case "kind":
      switch (value.kind) {
        case "live":
          return <IconLivePhoto {...props} />;
        case "hdr":
          return <IconHdr {...props} />;
        case "screenshot":
          return <IconScreenshot {...props} />;
        case "panorama":
          return <IconPanoramaHorizontal {...props} />;
        case "selfie":
          return <IconUserCircle {...props} />;
        case "silent":
          return <IconVolumeOff {...props} />;
        default:
          return <IconCamera {...props} />;
      }
    case "favorites":
      return <IconStarFilled {...props} />;
    case "rating":
      return <IconStar {...props} />;
    case "flag":
      return <IconFlag {...props} />;
    case "exif":
      switch (value.field) {
        case "aperture":
          return <IconAperture {...props} />;
        case "software":
          return <IconAppWindow {...props} />;
        case "videoCodec":
          return <IconMovie {...props} />;
        case "audioCodec":
          return <IconVolume {...props} />;
        case "hdrFormat":
          return <IconHdr {...props} />;
        case "colorSpace":
        case "bitDepth":
          return <IconPalette {...props} />;
        default:
          return <IconCamera {...props} />;
      }
    case "extension":
      return <IconFile {...props} />;
    case "text":
      return <IconSearch {...props} />;
    case "hasText":
    case "noText":
    case "photoText":
      return <IconTextRecognition {...props} />;
    case "range":
      switch (value.measure) {
        case "duration":
          return <IconClockHour4 {...props} />;
        case "fileSize":
          return <IconDatabase {...props} />;
        case "pixels":
        case "videoSize":
          return <IconDimensions {...props} />;
        case "frameRate":
          return <IconMovie {...props} />;
        case "altitude":
          return <IconMountain {...props} />;
      }
      break;
    case "shape":
      return value.shape === "portrait" ? (
        <IconRectangleVertical {...props} />
      ) : value.shape === "square" ? (
        <IconSquare {...props} />
      ) : (
        <IconRectangle {...props} />
      );
    case "lumen":
      return value.mark === "edited" ? <IconWand {...props} /> : value.mark === "developed" ? <IconAdjustments {...props} /> : <IconFileExport {...props} />;
  }
}

/** *in Trips › Europe* after an album's or collection's name, so two *Italy* albums differ (Round 19 §4). */
export function TagWhere({ value }: { value: TagValue }) {
  if (value.type !== "album" && value.type !== "collection") return null;
  return <TagWhereOf value={value} />;
}

function TagWhereOf({ value }: { value: TagValue & { type: "album" | "collection" } }) {
  const { collections } = useAlbumCollections();
  const albums = useAlbums();
  const path =
    value.type === "album"
      ? albumPath(collections, { collectionId: albums.albums.find((a) => a.id === value.id)?.collectionId ?? null })
      : collectionPath(collections, value.id)
          .slice(0, -1)
          .map((c) => c.name);
  return path.length > 0 ? <span className="tag-where"> in {path.join(" › ")}</span> : null;
}

/** The hints an empty, unfocused box rotates through, so people learn what it can find (Round 15). */
export const PLACEHOLDERS = ['Search… try "file created 2024"', 'Search… try "taken last week"', 'Search… try "length > 1 min"'];

/** How long each hint shows. */
const HINT_MS = 6_000;

/** The hint to show: `fixed` when given; else rotating while `idle`, the first with reduced motion. */
function useRotatingHint(fixed: string | undefined, idle: boolean, still: boolean): string {
  const [i, setI] = useState(0);
  useEffect(() => {
    if (fixed || !idle || still) return;
    const t = setInterval(() => setI((n) => (n + 1) % PLACEHOLDERS.length), HINT_MS);
    return () => clearInterval(t);
  }, [fixed, idle, still]);
  return fixed ?? PLACEHOLDERS[still ? 0 : i];
}

/** The typed text itself, always offered last, built here so it follows every keystroke. */
function textSuggestion(input: string): TagSuggestion {
  const text = input.trim();
  return {
    tag: { label: text, value: { type: "text", text } },
    field: "Text",
    count: null,
  };
}

/** Where a search's tags live: the page's URL by default, or a store of the caller's own (the smart album editor, Round 19 §8). */
export type { TagStore };

/** The text looks like a sentence of tags, not one term: `live and hdr`, `(a or b)`, `-screenshot`. */
export const sentenceLike = (input: string): boolean => /(^|\s)(and|or|not)(\s|$)|[()]|(^|\s)-[\p{L}\p{N}("]/iu.test(input);

/** What the *Try* row of an empty box fills in (it does not commit). */
const SENTENCE_EXAMPLE = "live or hdr not screenshot";

/** A list row: a suggestion, the *Read as* row of a typed sentence, or the sentence example that fills the box. */
type Option = TagSuggestion & { reading?: SentenceReading; fill?: string };

/**
 * The search field of a grid page (Round 7): typing lists tags for anything the library
 * knows; ↑↓ choose, Enter or Tab adds, Backspace on an empty field removes the last tag, Esc
 * closes the list and then clears the field. Chosen tags show in the [`TagBar`]. A typed
 * sentence (`live and hdr not screenshot`) is read into tags first (Round 20 §3); `portal`
 * draws the list in a portal, so a modal cannot clip it.
 */
export function TagSearchField({ placeholder, store, inputId = "global-search", portal = false }: { placeholder?: string; store?: TagStore; inputId?: string; portal?: boolean }) {
  const own = useTagSearch();
  const { tags, add, append, removeLast } = store ?? own;
  const [input, setInput] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const listId = useId();
  const empty = !input.trim();
  const { data: read } = useSentenceReading(input, open && sentenceLike(input));
  // `keepPreviousData` may hold the last sentence's reading: only text that still looks like one has it.
  const reading = sentenceLike(input) && read ? read : null;
  // With a reading, the suggestions are for its last term; with only an operator typed there is none.
  const term = reading ? reading.term : input;
  const termEmpty = !term.trim();
  // Focused and empty: the *Try* starters (newest year of each date field, …).
  const { data } = useTagSuggestions(term, open && !(reading && termEmpty));
  const { settings } = useSettings();
  const hint = useRotatingHint(placeholder, empty && !open, settings?.appearance.reduceMotion ?? false);
  const options = useMemo<Option[]>(() => {
    const fresh = (data ?? []).filter((s) => s.tag.value.type !== "text" && !tags.some((t) => sameTag(t, s.tag)));
    // `keepPreviousData` may still hold the last typed text's list: an empty box shows starters only.
    if (empty) {
      const example: Option = { tag: { label: SENTENCE_EXAMPLE, value: { type: "text", text: SENTENCE_EXAMPLE } }, field: "Try", count: null, fill: SENTENCE_EXAMPLE };
      return [...fresh.filter((s) => s.field === "Try"), example];
    }
    const rows: Option[] = reading && termEmpty ? [] : fresh.filter((s) => s.field !== "Try");
    const readAs: Option[] = reading && reading.seq.items.length > 0 ? [{ tag: { label: describe(reading.seq), value: { type: "text", text: input.trim() } }, field: "Read as", count: null, reading }] : [];
    return [...readAs, ...rows, textSuggestion(input)];
  }, [data, empty, input, tags, reading, termEmpty]);
  useEffect(() => setActive(0), [input]);
  const listRef = useRef<HTMLUListElement>(null);
  useEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  const showing = open && options.length > 0;
  const choose = (s: Option | undefined) => {
    if (!s) return;
    if (s.fill) {
      setInput(s.fill);
      return;
    }
    if (s.reading) append(s.reading.seq, s.reading.lead);
    // A suggestion under a reading takes the place of its last term.
    else if (reading && !termEmpty && s.tag.value.type !== "text") append(replaceLastTag(reading.seq, s.tag), reading.lead);
    else add(s.tag);
    setInput("");
  };

  const field = (
    <label className="search-field tag-search__field" data-open={showing}>
      <IconSearch size={16} className="search-field__icon" aria-hidden />
      <input
        id={inputId}
        className="search-field__input"
        type="search"
        role="combobox"
        aria-label="Search"
        aria-autocomplete="list"
        aria-expanded={showing}
        aria-controls={listId}
        aria-activedescendant={showing ? `${listId}-${active}` : undefined}
        placeholder={hint}
        autoComplete="off"
        spellCheck={false}
        value={input}
        onChange={(e) => {
          setInput(e.currentTarget.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          const n = options.length;
          switch (e.key) {
            case "ArrowDown":
              if (!n) return;
              e.preventDefault();
              setOpen(true);
              setActive((a) => (showing ? (a + 1) % n : 0));
              break;
            case "ArrowUp":
              if (!n) return;
              e.preventDefault();
              setActive((a) => (a - 1 + n) % n);
              break;
            case "Enter":
              if (!showing) return;
              e.preventDefault();
              choose(options[active]);
              break;
            case "Tab":
              // Tab completes, as in a terminal, but only while there is something to take.
              if (!showing || e.shiftKey) return;
              e.preventDefault();
              choose(options[active]);
              break;
            case "Backspace":
              if (input === "" && tags.length) {
                e.preventDefault();
                removeLast();
              }
              break;
            case "Escape":
              e.preventDefault();
              if (showing) setOpen(false);
              else {
                setInput("");
                e.currentTarget.blur();
              }
              break;
          }
        }}
      />
      {input && (
        <button type="button" className="search-field__clear" aria-label="Clear search" onMouseDown={(e) => e.preventDefault()} onClick={() => setInput("")}>
          <IconX size={14} />
        </button>
      )}
    </label>
  );

  const body = (
    <>
      <ul ref={listRef} className="tag-search__list" role="listbox" id={listId} aria-label="Suggestions">
        {options.map((s, i) => (
          <Fragment key={`${s.field}:${s.fill ? "fill:" : ""}${tagKey(s.tag)}`}>
            {(i === 0 || options[i - 1].field !== s.field) && (
              <li className="tag-search__group" role="presentation">
                {s.field === "Text" ? "Search for" : s.field}
              </li>
            )}
            <li
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              aria-label={s.reading ? `Read as ${s.tag.label}` : undefined}
              className="tag-search__option"
              data-read={s.reading ? true : undefined}
              // Keep the focus in the field, so the list stays open under the pointer.
              onMouseDown={(e) => e.preventDefault()}
              onMouseMove={() => i !== active && setActive(i)}
              onClick={() => choose(s)}
            >
              {s.reading ? (
                <span className="tag-search__read" aria-hidden>
                  {s.reading.lead && (
                    <span className="tag-bar__join" data-strong={s.reading.lead === "or"}>
                      {s.reading.lead}
                    </span>
                  )}
                  <TokenRow seq={s.reading.seq} />
                </span>
              ) : (
                <>
                  <span className="tag-search__icon">
                    <TagIcon value={s.tag.value} />
                  </span>
                  <span className="tag-search__label">
                    {s.fill ? (
                      s.tag.label
                    ) : s.tag.value.type === "text" ? (
                      <>“{s.tag.label}”</>
                    ) : (
                      highlight(s.tag.label, term).map(([part, bold], k) =>
                        bold ? (
                          <b key={k} className="tag-search__match">
                            {part}
                          </b>
                        ) : (
                          <Fragment key={k}>{part}</Fragment>
                        ),
                      )
                    )}
                    <TagWhere value={s.tag.value} />
                  </span>
                  {s.count != null && <span className="tag-search__count">{s.count.toLocaleString()}</span>}
                </>
              )}
            </li>
          </Fragment>
        ))}
      </ul>
      <div className="tag-search__hint" aria-hidden>
        <span>
          <kbd className="tag-search__kbd">↑</kbd>
          <kbd className="tag-search__kbd">↓</kbd> choose
        </span>
        <span>
          <kbd className="tag-search__kbd">Enter</kbd> add
        </span>
        <span>
          <kbd className="tag-search__kbd">Esc</kbd> close
        </span>
        <span>and · or · not between tags</span>
      </div>
    </>
  );

  if (portal) {
    // In a portal, anchored to the field and as wide as it: a modal's edge cannot clip the list.
    return (
      <Popover opened={showing} position="bottom-start" width="target" offset={8} withinPortal transitionProps={{ duration: 120 }}>
        <Popover.Target>
          <div className="tag-search tag-search--portal">{field}</div>
        </Popover.Target>
        <Popover.Dropdown className="tag-search__popover">{body}</Popover.Dropdown>
      </Popover>
    );
  }
  return (
    <div className="tag-search">
      {field}
      <AnimatePresence>
        {showing && (
          <motion.div
            className="tag-search__dropdown"
            initial={{ opacity: 0, y: -6, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ duration: 0.14, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {body}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * The chosen tags, under the page's header (Round 7), drawn as a [`TokenRow`] (Round 20): the
 * joins between them are buttons. It opens and closes on a spring.
 */
export function TagBar({ store }: { store?: TagStore } = {}) {
  const own = useTagSearch();
  const s = store ?? own;
  return (
    <AnimatePresence initial={false}>
      {s.tags.length > 0 && (
        <motion.div
          className="tag-bar"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={SPRING}
        >
          <div className="tag-bar__inner" role="list" aria-label="Search tags">
            <TokenRow seq={s.seq} ops={s} />
            <motion.button layout type="button" className="tag-bar__clear" onClick={s.clear} transition={SPRING}>
              Clear
            </motion.button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
