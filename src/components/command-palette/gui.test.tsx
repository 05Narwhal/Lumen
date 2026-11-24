import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { mockCommand } from "@/test/tauri-mock";
import { CommandPalette } from "./gui";
import { openPalette, paletteStore } from "./store";

function Path() {
  return <p data-testid="path">{useLocation().pathname}</p>;
}

describe("CommandPalette", () => {
  it("filters and runs a navigation command", async () => {
    mockCommand("get_settings", () => null);
    render(
      <Providers>
        <MemoryRouter>
          <Routes>
            <Route
              path="*"
              element={
                <>
                  <Path />
                  <CommandPalette />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    openPalette();
    const input = await screen.findByRole("textbox", { name: "Command" });
    await userEvent.type(input, "memo");
    await userEvent.keyboard("{Enter}");
    expect(screen.getByTestId("path")).toHaveTextContent("/memories");
    expect(paletteStore.get().open).toBe(false);
  });

  describe("plugin contributions (Round 25 P4a)", () => {
    const plug = (id: string, label: string, over: Record<string, unknown> = {}) => ({
      id,
      name: `Plugin ${id}`,
      enabled: true,
      state: { type: "ready" },
      contributions: [
        { slot: "map.toolbar", label, icon: null, action: "open-review:places" },
        { slot: "viewer.menu", label, icon: null, action: "open-review:places" },
      ],
      ...over,
    });
    function mountPalette() {
      mockCommand("get_settings", () => null);
      mockCommand("get_plugin_developer_mode", () => false);
      render(
        <Providers>
          <MemoryRouter>
            <Routes>
              <Route path="*" element={<><Path /><CommandPalette /></>} />
            </Routes>
          </MemoryRouter>
        </Providers>,
      );
      openPalette();
    }

    it("lists a ready plugin's contribution once and runs it", async () => {
      mockCommand("list_plugins", () => [plug("jl.x", "Find places")]);
      mountPalette();
      const input = await screen.findByRole("textbox", { name: "Command" });
      await userEvent.type(input, "find places");
      expect(await screen.findAllByText("Find places")).toHaveLength(1);
      await userEvent.keyboard("{Enter}");
      expect(screen.getByTestId("path")).toHaveTextContent("/review/places");
    });

    it("lists nothing for a plugin that is off", async () => {
      mockCommand("list_plugins", () => [plug("jl.x", "Find places", { enabled: false, state: { type: "off" } })]);
      mountPalette();
      const input = await screen.findByRole("textbox", { name: "Command" });
      await userEvent.type(input, "find places");
      expect(screen.queryByText("Find places")).not.toBeInTheDocument();
    });
  });
});
