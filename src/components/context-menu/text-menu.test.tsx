import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { ContextMenuHost, useContextMenu } from "./gui";

const clipboard = { readText: vi.fn(async () => "pasted"), writeText: vi.fn(async (_: string) => {}) };

/** A search field inside a card that has a menu of its own, as fields in rows and cards do. */
function Card({ onChange, readOnly = false }: { onChange: (v: string) => void; readOnly?: boolean }) {
  const { open } = useContextMenu();
  const [value, setValue] = useState("beach");
  return (
    <div onContextMenu={(e) => open(e, [{ label: "Card action", onSelect: () => {} }])}>
      <input
        aria-label="Search"
        value={value}
        readOnly={readOnly}
        onChange={(e) => {
          setValue(e.target.value);
          onChange(e.target.value);
        }}
      />
    </div>
  );
}

function mount(props: { onChange?: (v: string) => void; readOnly?: boolean } = {}) {
  render(
    <Providers>
      <Card onChange={props.onChange ?? vi.fn()} readOnly={props.readOnly} />
      <ContextMenuHost />
    </Providers>,
  );
  return screen.getByRole("textbox", { name: "Search" }) as HTMLInputElement;
}

const item = (name: string) => screen.findByRole("menuitem", { name: new RegExp(`^${name}`) }, { timeout: 5000 });

describe("the text field menu", () => {
  beforeEach(() => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    clipboard.readText.mockClear();
    clipboard.writeText.mockClear();
  });
  afterEach(() => {
    Reflect.deleteProperty(navigator, "clipboard");
  });

  it("opens on a field instead of the card's menu around it", async () => {
    const field = mount();
    fireEvent.contextMenu(field);
    expect(await item("Paste")).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Card action" })).toBeNull();
  });

  it("pastes at the caret and tells React", async () => {
    const onChange = vi.fn();
    const field = mount({ onChange });
    field.setSelectionRange(2, 2);
    fireEvent.contextMenu(field);
    await userEvent.click(await item("Paste"));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("bepastedach"));
    expect(field.value).toBe("bepastedach");
  });

  it("disables Cut and Copy without a selection, and cuts one", async () => {
    const onChange = vi.fn();
    const field = mount({ onChange });
    field.setSelectionRange(0, 0);
    fireEvent.contextMenu(field);
    expect(await item("Cut")).toHaveAttribute("data-disabled", "true");
    expect(await item("Copy")).toHaveAttribute("data-disabled", "true");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: /^Cut/ })).toBeNull());
    field.setSelectionRange(0, 2);
    fireEvent.contextMenu(field);
    await userEvent.click(await item("Cut"));
    await waitFor(() => expect(clipboard.writeText).toHaveBeenCalledWith("be"));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("ach"));
  });

  it("offers a read-only field Copy and Select all only", async () => {
    const field = mount({ readOnly: true });
    field.setSelectionRange(0, 5);
    fireEvent.contextMenu(field);
    await item("Copy");
    const names = screen.getAllByRole("menuitem").map((m) => m.textContent?.replace(/Mod\+.*/, ""));
    expect(names).toEqual(["Copy", "Select all"]);
  });
});
