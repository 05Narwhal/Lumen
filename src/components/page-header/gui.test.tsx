import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { useGlobalHotkeys } from "@/hooks/use-hotkeys";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { PageHeader, SEARCH_PAUSE_MS } from "./gui";

function Where() {
  const l = useLocation();
  return <output data-testid="where">{l.pathname + l.search}</output>;
}

/** The app's window-level key handler, as `app.tsx` mounts it. */
function Hotkeys() {
  useGlobalHotkeys();
  return null;
}

// The real pages are different components, so going from one to the other draws the header anew.
const AlbumsPage = () => (
  <div className="albums-route">
    <PageHeader title="Albums" />
  </div>
);
const SearchPage = () => (
  <div className="search-route">
    <PageHeader title="Search" />
  </div>
);

function mount(path: string) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Hotkeys />
        <Routes>
          <Route path="/albums" element={<AlbumsPage />} />
          <Route path="/search" element={<SearchPage />} />
          <Route path="/people" element={<p>People page</p>} />
          <Route path="/slideshows" element={<p>Slideshows page</p>} />
        </Routes>
        <Where />
      </MemoryRouter>
    </Providers>,
  );
}

const where = () => decodeURIComponent(screen.getByTestId("where").textContent ?? "");
const pause = () => new Promise((r) => setTimeout(r, SEARCH_PAUSE_MS + 150));

describe("the top search box", () => {
  beforeEach(() => mockCommand("get_settings", () => null));

  // Round 19 §9: "6.7." typed on the Albums page landed on another screen. The pause after
  // "6" opens the results page, which drew a second search box; the typed text moved with the
  // old one, the focus did not, and the next digit ran as a navigation key.
  it("keeps the focus and the text through the pause that opens the results page", async () => {
    mount("/albums");
    const user = userEvent.setup();
    await user.type(screen.getByRole("searchbox", { name: "Search" }), "6");
    await pause();
    await waitFor(() => expect(where()).toBe("/search?q=6"));
    const box = screen.getByRole("searchbox", { name: "Search" });
    expect(box).toHaveFocus();
    await user.keyboard(".7.");
    expect(box).toHaveValue("6.7.");
    await pause();
    await waitFor(() => expect(where()).toBe("/search?q=6.7."));
  });
});
