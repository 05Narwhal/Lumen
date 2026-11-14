import { act, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { selectAll } from "@/hooks/use-selection";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { PhotoPicker } from "./gui";

describe("PhotoPicker", () => {
  it("counts the selection in its own element, apart from the buttons", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <PhotoPicker confirmLabel="Develop" onClose={vi.fn()} onPick={vi.fn()} />
      </Providers>,
    );
    act(() => selectAll([1, 2, 3]));
    expect(await screen.findByText("3 selected")).toHaveClass("photo-picker__count");
    expect(screen.getByRole("button", { name: "Develop 3" })).toBeEnabled();
  });

  // The confirm button's label was drawn in the muted grey on the accent (Round 4A): the
  // count's style was written for every `span` in the actions row, and a Mantine Button's
  // label is a span. Style the count by its class, never the row's descendants by tag.
  it("never styles the action row's descendants by tag", () => {
    const scss = readFileSync(join(__dirname, "styles.scss"), "utf8");
    const actions = scss.slice(scss.indexOf("&__actions"));
    expect(actions).not.toMatch(/^\s*(span|button|div|label)\s*[{,]/m);
  });
});
