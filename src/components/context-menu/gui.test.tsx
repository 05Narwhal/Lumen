import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { closeContextMenu, ContextMenuHost, openContextMenu, useContextMenu } from "./gui";

function Target({ onSelect }: { onSelect: () => void }) {
  const { open } = useContextMenu();
  return (
    <button
      type="button"
      onContextMenu={(e) =>
        open(e, [
          { label: "Open", onSelect },
          { divider: true },
          { label: "Move to Trash", danger: true, onSelect: vi.fn() },
        ])
      }
    >
      tile
    </button>
  );
}

describe("ContextMenu", () => {
  it("opens on right-click and runs the chosen item", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    const onSelect = vi.fn();
    render(
      <Providers>
        <Target onSelect={onSelect} />
        <ContextMenuHost />
      </Providers>,
    );
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("tile") });
    // Mantine's dropdown opens behind a portal and a 120 ms transition; a loaded CI box
    // can miss the default 1 s window, so wait explicitly.
    const item = await screen.findByRole("menuitem", { name: "Open" }, { timeout: 5000 });
    expect(screen.getByText("Move to Trash")).toBeInTheDocument();
    await userEvent.click(item);
    expect(onSelect).toHaveBeenCalled();
  });

  it("opens the focused object's menu with Shift+F10 at its corner, and Esc returns focus to it", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <Target onSelect={vi.fn()} />
        <ContextMenuHost />
      </Providers>,
    );
    const tile = screen.getByText("tile");
    tile.getBoundingClientRect = () => ({ left: 40, top: 100, right: 160, bottom: 220, width: 120, height: 120, x: 40, y: 100, toJSON: () => ({}) });
    tile.focus();
    await userEvent.keyboard("{Shift>}{F10}{/Shift}");
    expect(await screen.findByRole("menuitem", { name: "Open" }, { timeout: 5000 })).toBeInTheDocument();
    const anchor = document.querySelector<HTMLElement>(".context-menu__anchor")!;
    expect([anchor.style.left, anchor.style.top]).toEqual(["48px", "212px"]);
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: "Open" })).toBeNull());
    expect(document.activeElement).toBe(tile);
  });

  it("opens with the context-menu key too, and does nothing with nothing focused", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <Target onSelect={vi.fn()} />
        <ContextMenuHost />
      </Providers>,
    );
    await userEvent.keyboard("{ContextMenu}");
    expect(screen.queryByRole("menuitem", { name: "Open" })).toBeNull();
    screen.getByText("tile").focus();
    await userEvent.keyboard("{ContextMenu}");
    expect(await screen.findByRole("menuitem", { name: "Open" }, { timeout: 5000 })).toBeInTheDocument();
  });

  it("lets a menu with a submenu overflow its box (the submenu is drawn inside it), and keeps a flat one scrollable", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <ContextMenuHost />
      </Providers>,
    );
    act(() => openContextMenu(10, 10, [{ label: "Rate", children: [{ label: "One", onSelect: vi.fn() }] }]));
    const nested = (await screen.findByRole("menuitem", { name: "Rate" }, { timeout: 5000 })).closest('[role="menu"]');
    expect(nested).toHaveAttribute("data-nested");
    act(() => closeContextMenu());
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: "Rate" })).toBeNull());
    act(() => openContextMenu(10, 10, [{ label: "Open", onSelect: vi.fn() }]));
    const flat = (await screen.findByRole("menuitem", { name: "Open" }, { timeout: 5000 })).closest('[role="menu"]');
    expect(flat).not.toHaveAttribute("data-nested");
  });
});
