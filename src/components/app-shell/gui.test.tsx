import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand } from "@/test/tauri-mock";
import { AppShell, TitlebarPortal } from "./gui";

function mount(children: React.ReactNode) {
  mockCommand("get_settings", () => null);
  render(
    <Providers><MemoryRouter>
      <AppShell devices={[]} sidebarCollapsed={false} onToggleSidebar={vi.fn()}>
        {children}
      </AppShell>
    </MemoryRouter></Providers>,
  );
}

describe("AppShell", () => {
  // Round 26: the parts a theme.css names (THEMES.md *Parts*). The shell is a grid of named areas.
  it("marks the shell's parts and lays them out as grid areas", () => {
    mount(<p>content</p>);
    const part = (p: string) => document.querySelector(`[data-part="${p}"]`);
    for (const p of ["shell", "titlebar", "sidebar", "sidebar-item", "main", "toolbar", "window-controls"]) expect(part(p), p).not.toBeNull();
    expect(part("main")?.contains(screen.getByRole("main"))).toBe(true);
    // The title bar and the page are the shell's own children (the body is only a wrapper).
    expect(part("shell")?.contains(part("titlebar"))).toBe(true);
    expect(part("titlebar")?.contains(part("window-controls"))).toBe(true);
  });

  it("has a title bar and renders children in main", () => {
    mount(<p>content</p>);
    expect(document.querySelector(".app-shell__titlebar")).not.toBeNull();
    expect(screen.getByRole("main")).toHaveTextContent("content");
  });

  // Round 4J: a screen's toolbar (Develop's) sits in the title bar's row, before the window
  // controls and never under them.
  it("puts a screen's toolbar into the title bar, before the window controls", async () => {
    mount(<TitlebarPortal><button type="button">Export…</button></TitlebarPortal>);
    const button = await screen.findByRole("button", { name: "Export…" });
    const bar = document.querySelector(".app-shell__titlebar")!;
    expect(bar.contains(button)).toBe(true);
    expect(screen.getByRole("main").contains(button)).toBe(false);
    const controls = screen.getByRole("group", { name: "Window" });
    expect(button.compareDocumentPosition(controls) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(bar.getAttribute("data-toolbar")).toBe("true");
  });

  // Round 6: Settings takes the whole window.
  it("leaves the sidebar out when asked", () => {
    mockCommand("get_settings", () => null);
    render(
      <Providers><MemoryRouter>
        <AppShell devices={[]} sidebarCollapsed={false} onToggleSidebar={vi.fn()} sidebar={false}>
          <p>content</p>
        </AppShell>
      </MemoryRouter></Providers>,
    );
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(screen.getByRole("main")).toHaveTextContent("content");
  });

  it("draws a toolbar in place when there is no shell", () => {
    render(<TitlebarPortal><button type="button">Alone</button></TitlebarPortal>);
    expect(screen.getByRole("button", { name: "Alone" })).toBeInTheDocument();
  });
});
