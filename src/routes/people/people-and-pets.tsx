import { useSearchParams } from "react-router";
import { PageHeader } from "@/components/page-header";
import { Segmented } from "@/components/segmented";
import { usePeople } from "@/hooks/use-people";
import { usePets } from "@/hooks/use-pets";
import { PetsBody } from "@/routes/pets/index-page";
import { PeopleBody } from "./index-page";

type Show = "all" | "people" | "pets";
const TABS = [
  { label: "All", value: "all" as const },
  { label: "People", value: "people" as const },
  { label: "Pets", value: "pets" as const },
];

/**
 * `/people` (Round 20): people and pets on one page. *All* draws a heading with a count over
 * each body, people first; a tab draws one body alone. The tab lives in the URL (`?show=people`,
 * `?show=pets`; absent = all). Each body keeps its own state, menus and keys. An open group
 * (`?group=`) is a view of People alone, with its own header.
 */
export function PeopleAndPets() {
  const [params, setParams] = useSearchParams();
  const raw = params.get("show");
  const show: Show = raw === "people" || raw === "pets" ? raw : "all";
  const { people, isLoading: peopleLoading } = usePeople();
  const { pets, isLoading: petsLoading } = usePets();

  if (params.has("group")) return <PeopleBody />;

  const loaded = !peopleLoading && !petsLoading;
  // Nothing found at all, on the *All* view: today's People empty state, whole.
  const nothing = show === "all" && loaded && people.length === 0 && pets.length === 0;
  const tabs = <Segmented label="Show" size="sm" data={TABS} value={show} onChange={(v) => setParams(v === "all" ? {} : { show: v }, { replace: true })} />;
  const head = (title: string, n: number | null) => (
    <h2 className="people-and-pets__head">
      {title}
      {n != null && <span className="people-and-pets__n">{n.toLocaleString()}</span>}
    </h2>
  );

  return (
    <div className="people-and-pets">
      <PageHeader title="People & Pets" search={false} count={tabs} />
      {show === "people" || nothing ? (
        <PeopleBody />
      ) : show === "pets" ? (
        <PetsBody />
      ) : (
        <>
          <section aria-label="People">
            {head("People", peopleLoading ? null : people.length)}
            <PeopleBody compact />
          </section>
          <section aria-label="Pets">
            {head("Pets", petsLoading ? null : pets.length)}
            <PetsBody compact />
          </section>
        </>
      )}
    </div>
  );
}
