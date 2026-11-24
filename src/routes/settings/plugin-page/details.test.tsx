import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { PluginDetails } from "./details";

const plugin = (over: Partial<PluginInfo> = {}): PluginInfo => ({
  id: "acme.finder",
  name: "Finder",
  version: "1.0.0",
  author: "Acme",
  description: "",
  license: "MIT",
  trust: { type: "jlStudios" },
  kinds: ["tools"],
  permissions: [],
  enabled: true,
  dev: false,
  state: { type: "ready" },
  iconUrl: null,
  settings: [],
  sections: [],
  contributions: [
    { slot: "map.toolbar", label: "Review dates", icon: null, action: "open-review:date" },
    { slot: "palette", label: "Tidy up", icon: null, action: "tidy" },
  ],
  homepage: null,
  authorUrl: null,
  repository: null,
  screenshots: [],
  hasReadme: false,
  detectIds: [],
  assets: [],
  ...over,
});

function Where() {
  const l = useLocation();
  return <p data-testid="where">{l.pathname + l.search}</p>;
}

const show = (p: PluginInfo) =>
  render(
    <Providers>
      <MemoryRouter initialEntries={["/plugins"]}>
        <Routes>
          <Route
            path="*"
            element={
              <>
                <PluginDetails p={p} />
                <Where />
              </>
            }
          />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );

describe("a plugin page's contribution buttons", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    toastStore.set({ toasts: [] });
  });
  afterEach(cleanup);

  it("one button per thing the plugin adds to the app; a review opens its page", async () => {
    show(plugin());
    await userEvent.click(screen.getByRole("button", { name: "Review dates" }));
    expect(screen.getByTestId("where").textContent).toBe("/review/date?plugin=acme.finder");
  });

  it("a plugin's own action runs in the plugin and its answer is shown", async () => {
    mockCommand("plugin_ui_action", () => "Tidied 3 photos.");
    show(plugin());
    await userEvent.click(screen.getByRole("button", { name: "Tidy up" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("plugin_ui_action", { id: "acme.finder", action: "tidy" }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Tidied 3 photos."));
  });

  it("a plugin that is off or has no contributions shows no buttons", () => {
    show(plugin({ enabled: false }));
    expect(screen.queryByRole("button", { name: "Review dates" })).toBeNull();
    cleanup();
    show(plugin({ contributions: [] }));
    expect(screen.queryByRole("button", { name: "Tidy up" })).toBeNull();
  });
});
