import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { describe, expect, it } from "vitest";
import { isEmptyQuery, useSearch } from "./use-search";

function Probe() {
  const { query, text, setText, setFilters, clear, active } = useSearch();
  const { search } = useLocation();
  return (
    <div>
      <p data-testid="url">{search}</p>
      <p data-testid="text">{text}</p>
      <p data-testid="query">{JSON.stringify({ text: query.text, minRating: query.minRating, deviceIds: query.deviceIds })}</p>
      <p data-testid="active">{String(active)}</p>
      <input aria-label="q" value={text} onChange={(e) => setText(e.target.value)} />
      <button type="button" onClick={() => setFilters({ minRating: 4 })}>
        rate
      </button>
      <button type="button" onClick={() => setFilters({ minRating: null })}>
        unrate
      </button>
      <button type="button" onClick={clear}>
        clear
      </button>
    </div>
  );
}

const mount = (entry = "/library") =>
  render(
    <MemoryRouter initialEntries={[entry]}>
      <Probe />
    </MemoryRouter>,
  );

describe("useSearch", () => {
  it("debounces typing into ?q= and parses it back", async () => {
    mount();
    await userEvent.type(screen.getByLabelText("q"), "beach");
    // The input is instant; the URL waits for the debounce.
    expect(screen.getByTestId("text")).toHaveTextContent("beach");
    expect(screen.getByTestId("url")).toHaveTextContent("");
    await act(() => new Promise((r) => setTimeout(r, 260)));
    expect(screen.getByTestId("url")).toHaveTextContent("q=beach");
    expect(screen.getByTestId("query")).toHaveTextContent('"text":"beach"');
    expect(screen.getByTestId("active")).toHaveTextContent("true");
  });

  it("reads an existing query from the URL", () => {
    mount("/library?q=lisbon&filters=%7B%22minRating%22%3A3%7D");
    expect(screen.getByTestId("text")).toHaveTextContent("lisbon");
    expect(screen.getByTestId("query")).toHaveTextContent('"minRating":3');
  });

  it("adds and removes structured filters without touching the text", async () => {
    mount("/library?q=beach");
    await userEvent.click(screen.getByRole("button", { name: "rate" }));
    expect(screen.getByTestId("url")).toHaveTextContent("minRating");
    expect(screen.getByTestId("text")).toHaveTextContent("beach");
    await userEvent.click(screen.getByRole("button", { name: "unrate" }));
    expect(screen.getByTestId("url")).not.toHaveTextContent("filters=");
  });

  it("clears everything at once", async () => {
    mount("/library?q=beach&filters=%7B%22favorites%22%3Atrue%7D");
    expect(screen.getByTestId("active")).toHaveTextContent("true");
    await userEvent.click(screen.getByRole("button", { name: "clear" }));
    expect(screen.getByTestId("url")).not.toHaveTextContent("q=");
    expect(screen.getByTestId("active")).toHaveTextContent("false");
  });

  it("knows an empty query when it sees one", () => {
    const empty = { text: null, dateFrom: null, dateTo: null, deviceIds: [], mediaType: null, favorites: false, minRating: null, hasGps: null, hasText: false, noText: false, inText: [], camera: null, lens: null, kind: null, extension: null, includeRawPairs: false, colorLabels: [], flag: null, personIds: [], petIds: [], places: [], tags: [], months: [], albumIds: [], collectionIds: [], anyOf: [], searchTags: [] };
    expect(isEmptyQuery({ ...empty, hasText: true })).toBe(false);
    expect(isEmptyQuery(empty)).toBe(true);
    expect(isEmptyQuery({ ...empty, text: "   " })).toBe(true);
    expect(isEmptyQuery({ ...empty, minRating: 0 })).toBe(true);
    expect(isEmptyQuery({ ...empty, minRating: 1 })).toBe(false);
    expect(isEmptyQuery({ ...empty, deviceIds: [2] })).toBe(false);
    expect(isEmptyQuery({ ...empty, kind: "raw" })).toBe(false);
    expect(isEmptyQuery({ ...empty, personIds: [4] })).toBe(false);
    expect(isEmptyQuery({ ...empty, months: [3] })).toBe(false);
    expect(isEmptyQuery({ ...empty, searchTags: [{ label: "Videos", value: { type: "media", media: 1 } }] })).toBe(false);
  });
});
