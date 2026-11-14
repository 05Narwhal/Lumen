import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { PLACEHOLDERS, TagBar, TagSearchField } from "./gui";

function Here() {
  const l = useLocation();
  return <output data-testid="here">{l.search}</output>;
}

const settings = (reduceMotion: boolean) => ({
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion },
  import: {},
  playback: {},
  map: {},
  music: {},
  smart: {},
  export: {},
  search: { dateOrder: "auto" },
  window: {},
});

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/library"]}>
        <TagSearchField />
        <TagBar />
        <Here />
      </MemoryRouter>
    </Providers>,
  );
  return screen.getByRole("combobox", { name: "Search" });
}

/** The tags of the sequence in the URL, in reading order. */
const urlTags = (): { label: string }[] => {
  const raw = JSON.parse(new URLSearchParams(screen.getByTestId("here").textContent ?? "").get("tags") ?? "{}") as { items?: { tag: { label: string } }[] };
  return (raw.items ?? []).map((i) => i.tag);
};

describe("TagSearchField", () => {
  afterEach(() => vi.useRealTimers());

  // Round 15: an empty box offers starters, so people find the date fields.
  it("offers starters when focused with an empty box", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => settings(false));
    const created = { tag: { label: "File created: 2024", value: { type: "dates", from: 1_704_067_200, to: 1_735_689_599, field: "fileCreated" } }, field: "Try", count: 3 };
    const asked: string[] = [];
    mockCommand("suggest_tags", (a) => {
      const input = String((a as { input: string }).input);
      asked.push(input);
      return input === "" ? [created] : [];
    });
    const field = mount();
    await userEvent.click(field);
    expect(await screen.findByRole("option", { name: /File created: 2024/ })).toBeInTheDocument();
    expect(asked).toContain("");
    expect(screen.getByText("Try")).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /“/ })).toBeNull();
    await userEvent.keyboard("{Enter}");
    const tags = () => urlTags();
    await waitFor(() => expect(tags()).toHaveLength(1));
    expect(tags()[0].label).toBe("File created: 2024");
  });

  it("rotates its hint while empty", async () => {
    mockLibraryDefaults();
    mockCommand("suggest_tags", () => []);
    mockCommand("get_settings", () => settings(false));
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const field = mount();
    await waitFor(() => expect(field.getAttribute("placeholder")).toBe(PLACEHOLDERS[0]));
    act(() => void vi.advanceTimersByTime(6_000));
    expect(field.getAttribute("placeholder")).toBe(PLACEHOLDERS[1]);
  });

  it("keeps one hint with reduced motion", async () => {
    mockLibraryDefaults();
    mockCommand("suggest_tags", () => []);
    mockCommand("get_settings", () => settings(true));
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const field = mount();
    await act(async () => void (await vi.advanceTimersByTimeAsync(50)));
    const first = field.getAttribute("placeholder");
    act(() => void vi.advanceTimersByTime(13_000));
    expect(field.getAttribute("placeholder")).toBe(first);
  });

  // Round 11: a typed date after a field's name is one chip, the first suggestion.
  it("adds one chip for a typed date after a field", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => ({ version: 1, appearance: { theme: "gallery", scheme: "auto", tileSize: "m" }, import: {}, playback: {}, map: {}, music: {}, smart: {}, export: {}, search: { dateOrder: "auto" }, window: {} }));
    const day = { tag: { label: "Taken: 4 June 2026", value: { type: "dates", from: 1_780_531_200, to: 1_780_617_599 } }, field: "Date", count: null };
    mockCommand("suggest_tags", (a) => {
      const input = String((a as { input: string }).input);
      return input === "taken: 4.6.2026" ? [day, { tag: { label: input, value: { type: "text", text: input } }, field: "Text", count: null }] : [];
    });
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library"]}>
          <TagSearchField />
          <TagBar />
          <Here />
        </MemoryRouter>
      </Providers>,
    );
    const field = screen.getByRole("combobox", { name: "Search" });
    await userEvent.type(field, "taken: 4.6.2026");
    await screen.findByRole("option", { name: /Taken: 4 June 2026/ });
    await userEvent.keyboard("{Enter}");
    const tagsNow = () => urlTags();
    await waitFor(() => expect(tagsNow()).not.toEqual([]));
    const tags = tagsNow();
    expect(tags).toHaveLength(1);
    expect(tags[0].label).toBe("Taken: 4 June 2026");
  });
});
