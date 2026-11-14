import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { PageHeader } from "@/components/page-header";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { SearchOverview } from "@/types/generated/SearchOverview";
import { SearchRoute } from "./gui";

const chip = (kind: string, label: string, span: [number, number], token: string, alternatives = [], path: string[] = []) => ({ kind, label, span, token, path, alternatives });

/** What Rust answers for "anna beach 2023", with Florence as an ambiguous reading elsewhere. */
function overviewFor(q: string): SearchOverview {
  const empty = { chips: [], rest: q, people: [], pets: [], places: [], albums: [], memories: [], total: 0 };
  if (q === "anna beach 2023")
    return {
      ...empty,
      rest: "",
      total: 42,
      chips: [chip("person", "Anna", [0, 4], 'person:"Anna"'), chip("tag", "Beach", [5, 10], 'tag:"Beach"'), chip("date", "2023", [11, 15], "2023")],
      people: [{ id: 7, name: "Anna", faceCount: 3, photoCount: 30, coverFaceId: 70 }],
      places: [],
      albums: [{ id: 5, name: "Beach days", kind: 0, parentId: null, coverAssetId: 1, itemCount: 9, createdAt: 0, updatedAt: 0, query: null }],
    } as unknown as SearchOverview;
  if (q === "florence")
    return {
      ...empty,
      rest: "",
      total: 40,
      chips: [{ ...chip("place", "Florence, Italy", [0, 8], 'place:"Florence, Italy"'), alternatives: [chip("person", "Florence", [0, 8], 'person:"Florence"')] }],
      places: [{ place: { kind: "city", name: "Florence", country: "Italy" }, label: "Florence, Italy", photoCount: 40, lat: 43.77, lon: 11.25, coverAssetId: 3 }],
    } as unknown as SearchOverview;
  if (q === "milo beach")
    return {
      ...empty,
      rest: "",
      total: 12,
      chips: [chip("pet", "Milo", [0, 4], 'pet:"Milo"'), chip("tag", "Beach", [5, 10], 'tag:"Beach"')],
      pets: [{ id: 3, name: "Milo", species: "dog", detectionCount: 20, photoCount: 18, coverDetectionId: 30 }],
    } as unknown as SearchOverview;
  if (q === "italy album or ben")
    return {
      ...empty,
      rest: "",
      total: 9,
      chips: [
        chip("album", "Italy", [0, 11], 'album:"Trips › Europe › Italy"', [], ["Trips", "Europe"]),
        chip("op", "or", [12, 14], "or"),
        chip("person", "Ben", [15, 18], 'person:"Ben"'),
      ],
    } as unknown as SearchOverview;
  return empty as unknown as SearchOverview;
}

function Where() {
  const l = useLocation();
  return <output data-testid="where">{l.pathname + l.search}</output>;
}

function mount(path: string) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  mockCommand("search_overview", (a) => overviewFor(String(a?.q ?? "")));
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/search" element={<SearchRoute />} />
          <Route path="/library" element={<PageHeader title="Library" />} />
          <Route path="/people/:id" element={<p>Person page</p>} />
          <Route path="/pets/:id" element={<p>Pet page</p>} />
          <Route path="/albums/:id" element={<p>Album page</p>} />
        </Routes>
        <Where />
      </MemoryRouter>
    </Providers>,
  );
}

const where = () => decodeURIComponent(screen.getByTestId("where").textContent ?? "").replace(/\+/g, " ");
const SIZES = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;

describe("Search", () => {
  beforeEach(() => {
    for (const p of SIZES) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
  });
  afterEach(() => {
    for (const p of SIZES) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  it("goes to the results page on Enter and shows what the text was read as", async () => {
    mount("/library");
    await userEvent.type(screen.getByRole("searchbox", { name: "Search" }), "anna beach 2023{Enter}");
    await waitFor(() => expect(where()).toBe("/search?q=anna beach 2023"));
    const chips = await screen.findByRole("list", { name: "Search terms" });
    expect(within(chips).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Anna", "Beach", "2023"]);
    expect(screen.getByText("42 photos")).toBeInTheDocument();
  });

  it("an album says which collection it is in, and or / and / not show between the terms", async () => {
    mount("/search?q=italy%20album%20or%20ben");
    const chips = await screen.findByRole("list", { name: "Search terms" });
    expect(within(chips).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Italy in Trips › Europe", "or", "Ben"]);
    expect(screen.getByRole("button", { name: "Remove Italy (Trips › Europe)" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Remove / })).toHaveLength(2);
  });

  it("removing a chip rewrites the search", async () => {
    mount("/search?q=anna%20beach%202023");
    await userEvent.click(await screen.findByRole("button", { name: "Remove Beach" }));
    await waitFor(() => expect(where()).toBe("/search?q=anna 2023"));
  });

  it("an ambiguous word offers its other reading", async () => {
    mount("/search?q=florence");
    await userEvent.click(await screen.findByRole("button", { name: "Florence, Italy: other meanings" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /Florence.*person/ }));
    await waitFor(() => expect(where()).toBe('/search?q=person:"Florence"'));
  });

  it("shows the people, places and albums the search names, and hides empty sections", async () => {
    mount("/search?q=anna%20beach%202023");
    const people = await screen.findByRole("region", { name: "People" });
    expect(within(people).getByText("Anna")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Albums" })).toHaveTextContent("Beach days");
    expect(screen.queryByRole("region", { name: "Places" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Memories" })).toBeNull();
    await userEvent.click(within(people).getByRole("link", { name: /Anna/ }));
    expect(await screen.findByText("Person page")).toBeInTheDocument();
  });

  it("shows the pet a search names, with its own crop, linking to its page (Round 4C)", async () => {
    mount("/search?q=milo%20beach");
    const pets = await screen.findByRole("region", { name: "Pets" });
    const milo = within(pets).getByRole("link", { name: /Milo/ });
    expect(milo.querySelector("img")?.getAttribute("src")).toContain("pet/30");
    await userEvent.click(milo);
    expect(await screen.findByText("Pet page")).toBeInTheDocument();
  });

  it("a place card narrows the search to that place", async () => {
    mount("/search?q=florence");
    const places = await screen.findByRole("region", { name: "Places" });
    await userEvent.click(within(places).getByRole("link", { name: /Florence, Italy/ }));
    await waitFor(() => expect(where()).toBe('/search?q=place:"Florence, Italy"'));
  });

  it("sorts by best match unless asked for the newest first", async () => {
    mount("/search?q=anna%20beach%202023");
    await screen.findByText("42 photos");
    const scopes = () => invokeMock.mock.calls.filter(([c]) => c === "get_layout").map(([, a]) => (a as { scope: { bestMatch: boolean } }).scope);
    await waitFor(() => expect(scopes().at(-1)?.bestMatch).toBe(true));
    await userEvent.click(screen.getByRole("radio", { name: "Newest first" }));
    await waitFor(() => expect(scopes().at(-1)?.bestMatch).toBe(false));
    expect(where()).toContain("sort=newest");
  });

  it("says when nothing matches", async () => {
    mount("/search?q=red%20umbrella");
    const title = await screen.findByText(/No photos match/);
    expect(title).toHaveTextContent("No photos match “red umbrella”.");
    await userEvent.click(within(title.closest(".empty-state") as HTMLElement).getByRole("button", { name: "Clear search" }));
    await waitFor(() => expect(where()).toBe("/search"));
  });
});
