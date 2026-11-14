import { MotionGlobalConfig } from "motion/react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import { SmartAlbumEditor } from "./gui";

MotionGlobalConfig.skipAnimations = true;

describe("SmartAlbumEditor (Round 20 §4)", () => {
  it("shows a legacy draft's tags as chips, counts them, and saves them as a tagExpr", async () => {
    mockLibraryDefaults();
    mockCommand("count_scope", () => 12);
    mockCommand("scope_ids", () => [7, 8, 9]);
    const onSave = vi.fn();
    const query: AssetQuery = {
      ...EMPTY_QUERY,
      minRating: 3,
      searchTags: [{ label: "Live Photos", value: { type: "kind", kind: "live" } }],
    };
    render(
      <Providers>
        <MemoryRouter>
          <SmartAlbumEditor draft={{ id: 4, name: "Best of Italy", query }} onClose={() => {}} onSave={onSave} />
        </MemoryRouter>
      </Providers>,
    );

    // The old tag is a chip in the token row; the old filter is an *Also:* chip.
    expect(await screen.findByText("Live Photos")).toBeTruthy();
    expect(screen.getByText("3+ stars")).toBeTruthy();
    expect(await screen.findByText("12 match")).toBeTruthy();
    const counts = invokeMock.mock.calls.filter(([cmd]) => cmd === "count_scope");
    expect(counts.at(-1)?.[1]).toMatchObject({ scope: { query: { tagExpr: { items: [{ type: "tag" }] } } } });
    await waitFor(() => expect(screen.getByLabelText("Newest matching photos").querySelectorAll("li")).toHaveLength(3));

    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0][0];
    expect(saved).toMatchObject({ id: 4, name: "Best of Italy" });
    expect(saved.query).toMatchObject({ searchTags: [], text: null, minRating: 3 });
    expect(saved.query.tagExpr.items).toHaveLength(1);
    expect(saved.query.tagExpr.items[0].tag.label).toBe("Live Photos");
  });

  it("says what to do while there are no rules, and keeps Create album off", async () => {
    mockLibraryDefaults();
    render(
      <Providers>
        <MemoryRouter>
          <SmartAlbumEditor draft={{ name: "", query: EMPTY_QUERY }} onClose={() => {}} onSave={() => {}} />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Add a tag and the photos that match appear here.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Create album" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
