import { Button } from "@mantine/core";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { IconSearch } from "@tabler/icons-react";
import { useCallback, useMemo, type ReactNode } from "react";
import { Link, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { MemoryCard } from "@/components/memory-card";
import { MiniMap } from "@/components/mini-map";
import { PageHeader } from "@/components/page-header";
import { PhotoGrid } from "@/components/photo-grid";
import { SearchChips } from "@/components/search-chips";
import { Segmented } from "@/components/segmented";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { useSettings } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { removeSpan, replaceSpan } from "@/lib/search-text";
import { ROW_HEIGHTS } from "@/routes/library";
import type { Chip } from "@/types/generated/Chip";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { SearchOverview } from "@/types/generated/SearchOverview";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

type Sort = "best" | "newest";

const photos = (n: number) => `${n.toLocaleString()} ${n === 1 ? "photo" : "photos"}`;

/** A search for exactly one reading, as a link: a place card narrows the search to it. */
const searchFor = (token: string) => `/search?${new URLSearchParams({ q: token })}`;

function Section({ title, seeAll, children }: { title: string; seeAll?: string; children: ReactNode }) {
  return (
    <section className="search-route__section" aria-label={title}>
      <header className="search-route__section-head">
        <h2 className="search-route__section-title">{title}</h2>
        {seeAll && (
          <Link className="search-route__see-all" to={seeAll}>
            See all
          </Link>
        )}
      </header>
      <div className="search-route__row">{children}</div>
    </section>
  );
}

/**
 * `/search?q=` (Round 4K): what the text was read as, then the people, places, albums and
 * memories it names, then every photo it finds — best match first unless asked otherwise.
 */
export function SearchRoute() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const sort: Sort = params.get("sort") === "newest" ? "newest" : "best";
  const { settings } = useSettings();
  const rowHeight = ROW_HEIGHTS[settings?.appearance.tileSize ?? "m"];

  const overview = useQuery({
    queryKey: queryKeys.searchOverview(q),
    queryFn: () => invoke<SearchOverview>("search_overview", { q }),
    enabled: !!q.trim(),
    placeholderData: keepPreviousData,
  });
  const o = q.trim() ? overview.data : undefined;

  const scope = useMemo<LayoutScope>(
    () => ({ ...LIBRARY_SCOPE, bestMatch: sort === "best", query: { ...EMPTY_QUERY, text: q.trim() || null } }),
    [q, sort],
  );

  const setText = useCallback(
    (text: string) =>
      setParams((p) => {
        if (text.trim()) p.set("q", text);
        else p.delete("q");
        p.delete("asset");
        return p;
      }),
    [setParams],
  );
  const setSort = (s: Sort) =>
    setParams(
      (p) => {
        if (s === "newest") p.set("sort", "newest");
        else p.delete("sort");
        return p;
      },
      { replace: true },
    );
  const onOpen = useCallback(
    (id: number) =>
      setParams((p) => {
        p.set("asset", String(id));
        return p;
      }),
    [setParams],
  );

  const header = (
    <PageHeader
      title="Search"
      count={o ? photos(Number(o.total)) : undefined}
      toolbar={
        q.trim() && !(o && Number(o.total) === 0) ? (
          <Segmented<Sort>
            size="xs"
            label="Order"
            value={sort}
            onChange={setSort}
            data={[
              { label: "Best match", value: "best" },
              { label: "Newest first", value: "newest" },
            ]}
          />
        ) : undefined
      }
    />
  );

  if (!q.trim()) {
    return (
      <div className="search-route">
        {header}
        <EmptyState
          icon={<IconSearch />}
          title="Search your photos"
          description="Type a person, a place, a date or anything a photo shows. Try “Anna or Ben, not screenshots”."
          action={
            <Button component={Link} to="/library">
              Browse the library
            </Button>
          }
        />
      </div>
    );
  }

  const nothing = o && Number(o.total) === 0;
  return (
    <div className="search-route">
      {header}
      {o && (
        <div className="search-route__top">
          <SearchChips
            chips={o.chips}
            rest={o.rest}
            onRemove={(c: Chip) => setText(removeSpan(q, c.span))}
            onChoose={(c: Chip, alt: Chip) => setText(replaceSpan(q, c.span, alt.token))}
          />
          <div className="search-route__strip">
            {o.people.length > 0 && (
              <Section title="People" seeAll="/people">
                {o.people.map((p) => (
                  <Link key={p.id} className="search-route__person" to={`/people/${p.id}`}>
                    {p.coverFaceId != null ? <img className="search-route__face" src={mediaUrl("face", p.coverFaceId)} alt="" loading="lazy" /> : <span className="search-route__face" />}
                    <span className="search-route__name">{p.name}</span>
                    <span className="search-route__meta">{photos(Number(p.photoCount))}</span>
                  </Link>
                ))}
              </Section>
            )}
            {o.pets.length > 0 && (
              <Section title="Pets" seeAll="/people?show=pets">
                {o.pets.map((p) => (
                  <Link key={p.id} className="search-route__person" to={`/pets/${p.id}`}>
                    {p.coverDetectionId != null ? <img className="search-route__face search-route__face--pet" src={mediaUrl("pet", p.coverDetectionId)} alt="" loading="lazy" /> : <span className="search-route__face search-route__face--pet" />}
                    <span className="search-route__name">{p.name}</span>
                    <span className="search-route__meta">{photos(Number(p.photoCount))}</span>
                  </Link>
                ))}
              </Section>
            )}
            {o.places.length > 0 && (
              <Section title="Places" seeAll="/map">
                {o.places.map((p) => (
                  <Link key={p.label} className="search-route__card" to={searchFor(`place:"${p.label}"`)}>
                    <span className="search-route__card-media">
                      {p.lat != null && p.lon != null ? (
                        <MiniMap lat={p.lat} lon={p.lon} width={160} height={96} zoom={5} />
                      ) : p.coverAssetId != null ? (
                        <img className="search-route__cover" src={mediaUrl("thumb", p.coverAssetId)} alt="" loading="lazy" />
                      ) : null}
                    </span>
                    <span className="search-route__name">{p.label}</span>
                    <span className="search-route__meta">{photos(Number(p.photoCount))}</span>
                  </Link>
                ))}
              </Section>
            )}
            {o.albums.length > 0 && (
              <Section title="Albums" seeAll="/albums">
                {o.albums.map((a) => (
                  <Link key={a.id} className="search-route__card" to={`/albums/${a.id}`}>
                    <span className="search-route__card-media">{a.coverAssetId != null && <img className="search-route__cover" src={mediaUrl("thumb", a.coverAssetId)} alt="" loading="lazy" />}</span>
                    <span className="search-route__name">{a.name}</span>
                    <span className="search-route__meta">{photos(Number(a.itemCount))}</span>
                  </Link>
                ))}
              </Section>
            )}
            {o.memories.length > 0 && (
              <Section title="Memories" seeAll="/memories">
                {o.memories.map((m) => (
                  <div key={m.id} className="search-route__memory">
                    <MemoryCard memory={m} />
                  </div>
                ))}
              </Section>
            )}
          </div>
        </div>
      )}
      {nothing ? (
        <EmptyState
          icon={<IconSearch />}
          title={`No photos match “${q.trim()}”.`}
          description="Try fewer words, or another way of saying it."
          action={<Button onClick={() => setText("")}>Clear search</Button>}
        />
      ) : (
        <div className="search-route__grid">
          <PhotoGrid rowHeight={rowHeight} scope={scope} onOpen={onOpen} />
        </div>
      )}
    </div>
  );
}
