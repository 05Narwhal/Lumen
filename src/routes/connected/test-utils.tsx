import { render } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { ConnectedRoute } from "./gui";

/** Registers the app's default command mocks; tests register their own after this. */
export function mockDefaults() {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
}

/** Renders the connected-device route at `path` (`/connected/:kind/:id`). */
export function mount(path: string) {
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/connected/:kind/:id" element={<ConnectedRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}
