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

  const NORD = { plugin: "a.b", id: "nord", name: "Nord", tagline: "Arctic blues on slate.", base: "darkroom" as const, defaultScheme: "dark" };

  it("lists_plugin_themes_after_builtins", async () => {
    const onChange = vi.fn();
    render(<ThemePicker value={{ theme: "gallery", scheme: "light" }} onChange={onChange} pluginThemes={[NORD]} />);
    const names = screen.getAllByRole("radio").slice(0, 5).map((r) => r.textContent ?? "");
    expect(names[0]).toMatch(/Gallery/);
    expect(names[3]).toMatch(/Glass/);
    expect(names[4]).toMatch(/Nord/);
    expect(names[4]).toMatch(/Arctic blues on slate\./);
    // Picking it saves the Plugin choice and takes the theme's own scheme.
    await userEvent.click(screen.getByRole("radio", { name: /Nord/ }));
    expect(onChange).toHaveBeenCalledWith({ theme: { plugin: "a.b", theme: "nord" }, scheme: "dark" });
  });

  it("shows a plugin theme as picked, and clicking it again leaves the scheme alone", async () => {
    const onChange = vi.fn();
    render(
      <ThemePicker value={{ theme: { plugin: "a.b", theme: "nord" }, scheme: "light" }} onChange={onChange} pluginThemes={[NORD]} />,
    );
    expect(screen.getByRole("radio", { name: /Nord/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: /Gallery/ })).toHaveAttribute("aria-checked", "false");
    await userEvent.click(screen.getByRole("radio", { name: /Nord/ }));
    expect(onChange).toHaveBeenCalledWith({ theme: { plugin: "a.b", theme: "nord" }, scheme: "light" });
  });

  it("previews a plugin theme on its base with its plugin attribute", () => {
    const { container } = render(<ThemePicker value={{ theme: "gallery", scheme: "light" }} onChange={() => {}} pluginThemes={[NORD]} />);
    const preview = container.querySelector('[data-plugin-theme="a.b/nord"]');
    expect(preview).toHaveAttribute("data-theme", "darkroom");
  });
});
