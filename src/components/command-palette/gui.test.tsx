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
});
