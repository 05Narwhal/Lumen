import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ThemePicker } from "./gui";

describe("ThemePicker", () => {
  it("renders four themes and reports changes", async () => {
    const onChange = vi.fn();
    render(<ThemePicker value={{ theme: "gallery", scheme: "auto" }} onChange={onChange} />);
    expect(screen.getAllByRole("radio")).toHaveLength(7);
    expect(screen.getByRole("radio", { name: /Gallery/ })).toHaveAttribute("aria-checked", "true");
    await userEvent.click(screen.getByRole("radio", { name: /Paper/ }));
    expect(onChange).toHaveBeenCalledWith({ theme: "paper", scheme: "auto" });
    await userEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(onChange).toHaveBeenCalledWith({ theme: "gallery", scheme: "dark" });
  });

  const swatch = { bg: "#111", surface: "#222", sidebar: "#181818", fg: "#eee", fg2: "#aaa", accent: "#6cf", placeholder: "#333", radius: "8px", gap: "4px" };
  const NORD = {
    id: "a.b",
    name: "Nord",
    author: "Nobody",
    version: "1.0.0",
    description: "Arctic blues on slate.",
    license: "MIT",
    base: "darkroom" as const,
    defaultScheme: "dark",
    layout: true,
    hasIcon: false,
    tokens: { light: swatch, dark: swatch },
  };

  it("lists_installed_themes_after_builtins, with the Layout badge", async () => {
    const onChange = vi.fn();
    render(<ThemePicker value={{ theme: "gallery", scheme: "light" }} onChange={onChange} themes={[NORD]} />);
    const names = screen.getAllByRole("radio").slice(0, 5).map((r) => r.textContent ?? "");
    expect(names[0]).toMatch(/Gallery/);
    expect(names[3]).toMatch(/Glass/);
    expect(names[4]).toMatch(/Nord/);
    expect(names[4]).toMatch(/Layout/);
    expect(names[4]).toMatch(/Arctic blues on slate\./);
    // Picking it saves the theme choice and takes the theme's own scheme.
    await userEvent.click(screen.getByRole("radio", { name: /Nord/ }));
    expect(onChange).toHaveBeenCalledWith({ theme: { plugin: "a.b", theme: "theme" }, scheme: "dark" });
  });

  it("shows an installed theme as picked, and clicking it again leaves the scheme alone", async () => {
    const onChange = vi.fn();
    render(<ThemePicker value={{ theme: { plugin: "a.b", theme: "theme" }, scheme: "light" }} onChange={onChange} themes={[NORD]} />);
    expect(screen.getByRole("radio", { name: /Nord/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: /Gallery/ })).toHaveAttribute("aria-checked", "false");
    await userEvent.click(screen.getByRole("radio", { name: /Nord/ }));
    expect(onChange).toHaveBeenCalledWith({ theme: { plugin: "a.b", theme: "theme" }, scheme: "light" });
  });

  it("offers Remove and Install from file only when asked to", async () => {
    const onRemove = vi.fn();
    const onInstallFile = vi.fn();
    const { rerender } = render(<ThemePicker value={{ theme: "gallery", scheme: "light" }} onChange={() => {}} themes={[NORD]} />);
    expect(screen.queryByRole("button", { name: "Remove Nord" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Install a theme from file/ })).toBeNull();
    rerender(<ThemePicker value={{ theme: "gallery", scheme: "light" }} onChange={() => {}} themes={[NORD]} onRemove={onRemove} onInstallFile={onInstallFile} />);
    await userEvent.click(screen.getByRole("button", { name: "Remove Nord" }));
    expect(onRemove).toHaveBeenCalledWith(NORD);
    await userEvent.click(screen.getByRole("button", { name: /Install a theme from file/ }));
    expect(onInstallFile).toHaveBeenCalled();
  });
});
