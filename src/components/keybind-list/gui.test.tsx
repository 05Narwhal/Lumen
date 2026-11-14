import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { KeybindList } from "./gui";

const saved = () => invokeMock.mock.calls.filter(([c]) => c === "update_settings").map(([, a]) => (a as { settings: { keybinds: Record<string, string[]> } }).settings.keybinds);

async function setup() {
  mockCommand("get_settings", () => ({ version: 1, appearance: { reduceMotion: false, theme: "gallery", scheme: "auto" }, keybinds: {} }));
  mockCommand("update_settings", (a) => a?.settings);
  invokeMock.mockClear();
  render(
    <Providers>
      <KeybindList />
    </Providers>,
  );
  await screen.findByRole("button", { name: "Change the keys for Open the photo" });
  await new Promise((r) => setTimeout(r, 0)); // the settings query lands
}

describe("KeybindList", () => {
  it("records a key, saves it as the person's override and shows it", async () => {
    await setup();
    await userEvent.click(screen.getByRole("button", { name: "Change the keys for Open the photo" }));
    expect(screen.getByText("Press keys…")).toBeInTheDocument();
    await userEvent.keyboard("q");
    await waitFor(() => expect(saved().at(-1)).toEqual({ "grid.open": ["Q"] }));
    expect(screen.getByRole("button", { name: "Change the keys for Open the photo" })).toHaveTextContent("Q");
    // Back to the default clears the override.
    await userEvent.click(screen.getByRole("button", { name: "Reset Open the photo" }));
    await waitFor(() => expect(saved().at(-1)).toEqual({}));
  });

  it("offers Replace when the key is taken, and Esc cancels a recording", async () => {
    await setup();
    const open = () => screen.getByRole("button", { name: "Change the keys for Open the photo" });
    await userEvent.click(open());
    await userEvent.keyboard("{Escape}");
    expect(saved()).toHaveLength(0);
    await userEvent.click(open());
    await userEvent.keyboard("f");
    expect(await screen.findByRole("alert")).toHaveTextContent("already used by Add the selection to Favorites");
    expect(saved()).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Replace" }));
    await waitFor(() => expect(saved().at(-1)).toEqual({ "grid.favorite": [], "grid.open": ["F"] }));
  });
});
