import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { Sidebar } from "@/components/sidebar";
import { Splash } from "@/components/splash";
import { mockCommand } from "@/test/tauri-mock";
import { BrandMark } from "./gui";

// Round 8B: the app's logo inside Lumen — the title bar's left end, the splash and About.
const marks = () => [...document.querySelectorAll<HTMLImageElement>("img.brand-mark")];

describe("BrandMark", () => {
  it("is the logo at the size asked, decorative beside the name", () => {
    render(<BrandMark size={16} />);
    const [img] = marks();
    expect(img.getAttribute("src")).toMatch(/lumen-64\.png/);
    expect(img).toHaveAttribute("width", "16");
    expect(img).toHaveAttribute("alt", "");
  });

  it("takes the large logo above 32 px", () => {
    render(<BrandMark size={96} />);
    expect(marks()[0].getAttribute("src")).toMatch(/lumen-512\.png/);
  });

  it("sits at the title bar's left end beside the name; the 56 px rail keeps only its toggle", () => {
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
    const { unmount } = render(
      <Providers>
        <MemoryRouter>
          <Sidebar collapsed={false} onToggle={vi.fn()} devices={[]} />
        </MemoryRouter>
      </Providers>,
    );
    expect(document.querySelector(".sidebar__top img.brand-mark")).not.toBeNull();
    expect(screen.getByText("Lumen")).toBeInTheDocument();
    unmount();
    render(
      <Providers>
        <MemoryRouter>
          <Sidebar collapsed onToggle={vi.fn()} devices={[]} />
        </MemoryRouter>
      </Providers>,
    );
    expect(document.querySelector(".sidebar__top img.brand-mark")).toBeNull();
  });

  it("opens the splash", () => {
    render(<Splash visible />);
    expect(document.querySelector(".splash img.brand-mark")).not.toBeNull();
  });
});
