import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { Toaster } from "@/components/toast";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { DuplicateSet } from "@/types/generated/DuplicateSet";
import { DuplicatesCard, DuplicatesRoute } from "./gui";

const member = (id: number, extra: Partial<DuplicateSet["members"][number]> = {}) => ({
  id,
  width: 4032,
  height: 3024,
  bytes: 2_400_000,
  capturedAt: 1_700_000_000,
  sharpness: 0.5,
  rating: 0,
  favorite: false,
  edited: false,
  albums: 0,
  format: "JPG",
  ...extra,
});
const SETS: DuplicateSet[] = [
  { key: "1,2", likeness: "exact", members: [member(1, { format: "HEIC" }), member(2)], keep: 1 },
  { key: "3,4,5", likeness: "similar", members: [member(4, { rating: 3 }), member(3), member(5, { favorite: true })], keep: 4 },
];
const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

function mount(path = "/duplicates") {
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/duplicates" element={<DuplicatesRoute />} />
          <Route path="/" element={<DuplicatesCard />} />
        </Routes>
        <Where />
        <Toaster />
      </MemoryRouter>
    </Providers>,
  );
}

describe("duplicates", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    toastStore.set({ toasts: [] });
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_duplicate_sets", () => SETS);
    mockCommand("count_duplicate_sets", () => [2, 3]);
    mockCommand("resolve_duplicates", (a) => (a?.sets as { keep: number; remove: number[] }[]).map((s) => ({ keep: s.keep, removed: s.remove, favorite: false, rating: 0, colorLabel: 0, flag: 0, keywords: null, title: null, caption: null, albumsAdded: [] })));
    mockCommand("undo_resolve_duplicates", () => null);
    mockCommand("dismiss_duplicates", () => null);
    mockCommand("undo_dismiss_duplicates", () => null);
  });

  it("shows a Home card with the counts that opens the review", async () => {
    mount("/");
    expect(await screen.findByText("2 sets of duplicates")).toBeInTheDocument();
    expect(screen.getByText("3 photos could go")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Review" }));
    expect(screen.getByTestId("where")).toHaveTextContent("/duplicates");
  });

  it("shows no card when there are no duplicates", async () => {
    mockCommand("count_duplicate_sets", () => [0, 0]);
    mount("/");
    await waitFor(() => expect(calls("count_duplicate_sets")).toHaveLength(1));
    expect(screen.queryByText(/sets? of duplicates/)).toBeNull();
  });

  it("explains each kind in the badge's tooltip", async () => {
    mount();
    const exact = await screen.findByText("Exact copy");
    await userEvent.hover(exact);
    expect(await screen.findByText(/The same picture stored more than once/)).toBeInTheDocument();
    await userEvent.hover(screen.getByText("Similar"));
    expect(await screen.findByText(/Taken moments apart and almost identical/)).toBeInTheDocument();
  });

  it("keeps the photo clicked and removes the others, with Undo", async () => {
    mount();
    const set = await screen.findByRole("group", { name: "Similar: 3 photos" });
    expect(within(set).getByRole("button", { name: /Photo 4/ })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(within(set).getByRole("button", { name: /Photo 5/ }));
    expect(within(set).getByRole("button", { name: /Photo 5/ })).toHaveAttribute("aria-pressed", "true");
    expect(within(set).getByRole("button", { name: /Photo 4/ })).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(within(set).getByRole("button", { name: "Remove 2" }));
    await waitFor(() => expect(calls("resolve_duplicates")).toHaveLength(1));
    expect(calls("resolve_duplicates")[0]).toEqual({ sets: [{ keep: 5, remove: [3, 4] }] });
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(calls("undo_resolve_duplicates")).toHaveLength(1));
  });

  it("removes every suggested photo at once", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Remove all suggested (3)" }));
    await waitFor(() => expect(calls("resolve_duplicates")).toHaveLength(1));
    expect(calls("resolve_duplicates")[0]).toEqual({
      sets: [
        { keep: 1, remove: [2] },
        { keep: 4, remove: [3, 5] },
      ],
    });
    expect(await screen.findByText("Moved 3 photos to the Trash")).toBeInTheDocument();
  });

  it("removes only the exact copies at once", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Remove exact copies (1)" }));
    await waitFor(() => expect(calls("resolve_duplicates")).toHaveLength(1));
    expect(calls("resolve_duplicates")[0]).toEqual({ sets: [{ keep: 1, remove: [2] }] });
    expect(await screen.findByText("Moved 1 photo to the Trash")).toBeInTheDocument();
  });

  it("remembers a set that is not duplicates, with Undo", async () => {
    mount();
    const set = await screen.findByRole("group", { name: "Exact copy: 2 photos" });
    await userEvent.click(within(set).getByRole("button", { name: "Not duplicates" }));
    await waitFor(() => expect(calls("dismiss_duplicates")).toEqual([{ ids: [1, 2] }]));
    await act(async () => screen.getByRole("button", { name: "Undo" }).click());
    await waitFor(() => expect(calls("undo_dismiss_duplicates")).toEqual([{ ids: [1, 2] }]));
  });

  it("says so when there is nothing to review", async () => {
    mockCommand("list_duplicate_sets", () => []);
    mount();
    expect(await screen.findByText("No duplicates found.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Back to Home" }));
    expect(screen.getByTestId("where")).toHaveTextContent("/");
  });
});
