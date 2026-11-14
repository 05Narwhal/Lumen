import { IconChevronLeft, IconSearch, IconX } from "@tabler/icons-react";
import { Link, useLocation, useNavigate, useSearchParams } from "react-router";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { TagSearchField } from "@/components/tag-search";
import { useSearch } from "@/hooks/use-search";
import "./styles.scss";

/** How long typing pauses before the results page opens (Round 4K). */
export const SEARCH_PAUSE_MS = 300;

/** The bare search box, for pages that run their own search (the Albums page, Round 19 §9). */
export function Field({ text, setText, clear, onEnter, placeholder }: { text: string; setText: (t: string) => void; clear: () => void; onEnter?: () => void; placeholder: string }) {
  return (
    <label className="search-field">
      <IconSearch size={16} className="search-field__icon" aria-hidden />
      <input
        id="global-search"
        className="search-field__input"
        type="search"
        placeholder={placeholder}
        aria-label="Search"
        value={text}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onEnter?.();
          if (e.key === "Escape") {
            e.preventDefault();
            clear();
            e.currentTarget.blur();
          }
        }}
      />
      {text.trim() && (
        <button type="button" className="search-field__clear" aria-label="Clear search" onClick={clear}>
          <IconX size={14} />
        </button>
      )}
    </label>
  );
}

/** A search that narrows this page's grid in place (a device's photos in the Library). */
function InPlaceSearchField({ placeholder }: { placeholder: string }) {
  const { text, setText, clear } = useSearch();
  return <Field text={text} setText={setText} clear={clear} placeholder={placeholder} />;
}

/**
 * Set when the search box opens the results page while it has the focus. The results page draws
 * its own box, so the focus would fall to the page and the next key typed ("6.7." on the Albums
 * page) would run as a shortcut (`6` = Slideshows); the new box takes the focus over instead.
 */
let handOverFocus = false;

/**
 * The search box. Enter, or a pause in typing, opens the results page (`/search?q=…`); on
 * that page it rewrites the search in place. The text lives in the URL, so Back and a reload
 * keep it.
 */
function ResultsSearchField({ placeholder }: { placeholder: string }) {
  const navigate = useNavigate();
  const location = useLocation();
  const [params] = useSearchParams();
  const onResults = location.pathname === "/search";
  const urlText = onResults ? (params.get("q") ?? "") : "";
  const [text, setText] = useState(urlText);
  // A chip removed or a reading chosen rewrites the URL; the field follows it.
  useEffect(() => setText(urlText), [urlText]);

  useEffect(() => {
    if (!handOverFocus) return;
    handOverFocus = false;
    const input = document.getElementById("global-search") as HTMLInputElement | null;
    if (!input) return;
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, []);

  const go = useCallback(
    (t: string) => {
      if (t.trim() === urlText.trim()) return;
      if (!t.trim() && !onResults) return;
      const next = new URLSearchParams(onResults ? params : undefined);
      if (t.trim()) next.set("q", t);
      else next.delete("q");
      next.delete("asset");
      const qs = next.toString();
      if (document.activeElement?.id === "global-search") handOverFocus = true;
      navigate(`/search${qs ? `?${qs}` : ""}`, { replace: onResults });
    },
    [navigate, onResults, params, urlText],
  );
  useEffect(() => {
    if (text === urlText) return;
    const t = setTimeout(() => go(text), SEARCH_PAUSE_MS);
    return () => clearTimeout(t);
  }, [text, urlText, go]);

  const clear = () => {
    setText("");
    if (onResults) go("");
  };
  return <Field text={text} setText={setText} clear={clear} onEnter={() => go(text)} placeholder={placeholder} />;
}

export function SearchField({ placeholder = "Search", inPlace = false }: { placeholder?: string; inPlace?: boolean }) {
  return inPlace ? <InPlaceSearchField placeholder={placeholder} /> : <ResultsSearchField placeholder={placeholder} />;
}

/** Reports whether the nearest scrolling ancestor has scrolled past the top. */
function useScrolled(ref: React.RefObject<HTMLElement | null>) {
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let parent: HTMLElement | null = el.parentElement;
    while (parent && getComputedStyle(parent).overflowY !== "auto" && getComputedStyle(parent).overflowY !== "scroll") {
      parent = parent.parentElement;
    }
    if (!parent) return;
    const scroller = parent;
    const onScroll = () => setScrolled(scroller.scrollTop > 4);
    onScroll();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, [ref]);
  return scrolled;
}

export function PageHeader({
  title,
  search = true,
  toolbar,
  count,
  back,
  searchField,
}: {
  title: ReactNode;
  /** A way back to the page this one belongs to, shown before the title. */
  back?: { to: string; label: string };
  /**
   * `"tags"` filters this page's grid by search tags (Round 7); `"in-place"` narrows it by
   * text; `true` opens the results page.
   */
  search?: boolean | "in-place" | "tags";
  toolbar?: ReactNode;
  /** Short status beside the title, e.g. "37 of 1,248 match". */
  count?: ReactNode;
  /** A page's own search box, in place of `search`. */
  searchField?: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const scrolled = useScrolled(ref);
  return (
    <header ref={ref} className="page-header" data-scrolled={scrolled}>
      <div className="page-header__left">
        {back && (
          <Link className="page-header__back" to={back.to} aria-label={back.label} title={back.label}>
            <IconChevronLeft size={18} aria-hidden />
          </Link>
        )}
        <h1 className="page-header__title">{title}</h1>
      </div>
      {count && <span className="page-header__count">{count}</span>}
      <div className="page-header__right">
        {searchField ?? (search === "tags" ? <TagSearchField /> : search && <SearchField inPlace={search === "in-place"} />)}
        {toolbar && <div className="page-header__toolbar">{toolbar}</div>}
      </div>
    </header>
  );
}
