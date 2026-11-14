import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { chipsFor, FilterChips } from "./gui";

describe("chipsFor", () => {
  it("names a kind and clears only the kind", () => {
    const chips = chipsFor({ ...EMPTY_QUERY, kind: "raw", favorites: true }, () => "");
    const kind = chips.find((c) => c.key === "kind")!;
    expect(kind.label).toBe("RAW");
    expect(kind.clear).toEqual({ kind: null });
    expect(chips.map((c) => c.key)).toContain("favorites");
  });

  it("names Has text and clears only it (Round 13-3)", () => {
    const chip = chipsFor({ ...EMPTY_QUERY, hasText: true }, () => "").find((c) => c.key === "hasText")!;
    expect(chip.label).toBe("Has text");
    expect(chip.clear).toEqual({ hasText: false });
  });

  it("names No text and each Text rule, and clears only that one (Round 18 C2)", () => {
    const q = { ...EMPTY_QUERY, noText: true, inText: [{ text: "menu", exact: false }, { text: "pri mostu", exact: true }] };
    const chips = chipsFor(q, () => "");
    expect(chips.find((c) => c.key === "noText")).toMatchObject({ label: "No text", clear: { noText: false } });
    const menu = chips.find((c) => c.key === "inText:menu")!;
    expect(menu.label).toBe("Text: menu");
    expect(menu.clear).toEqual({ inText: [{ text: "pri mostu", exact: true }] });
    expect(chips.find((c) => c.key === "inText:\"pri mostu")!.label).toBe("Text: “pri mostu”");
  });
});

/**
 * The popover's own controls. These went untested because Mantine's `Select` cannot be
 * opened under jsdom at all — which is also why its dropdown, portalled to the body and so
 * *outside* the popover, counted as an outside click and shut the popover the moment you
 * picked anything.
 *
 * The dropdown keeps `display: none` under jsdom — Mantine's transition never runs — so
 * `getByRole` prunes everything inside it from the accessibility tree. Reach the controls
 * by label instead; they are in the document, which is what these tests are about.
 */
describe("the Filters popover", () => {
  function mount(query = EMPTY_QUERY) {
    mockLibraryDefaults();
    const onChange = vi.fn();
    render(
      <Providers>
        <FilterChips query={query} onChange={onChange} placement="dock" />
      </Providers>,
    );
    return onChange;
  }

  it("opens when the button is pressed", async () => {
    mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    expect(await screen.findByRole("combobox", { name: "Kind" })).toBeInTheDocument();
  });

  it("narrows to videos, and stays open to take another filter", async () => {
    const onChange = mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    await userEvent.click(await screen.findByRole("combobox", { name: "Kind" }));
    await userEvent.click(await screen.findByRole("option", { name: "Videos" }));
    expect(onChange).toHaveBeenCalledWith({ mediaType: 1, kind: null });
    // Still open: choosing one filter must not dismiss the sheet.
    expect(screen.getByRole("combobox", { name: "Location" })).toBeInTheDocument();
  });

  it("picks a kind, a location and a rating", async () => {
    const onChange = mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    await userEvent.click(await screen.findByRole("combobox", { name: "Kind" }));
    await userEvent.click(await screen.findByRole("option", { name: "RAW" }));
    expect(onChange).toHaveBeenCalledWith({ kind: "raw", mediaType: null });

    await userEvent.click(screen.getByRole("combobox", { name: "Location" }));
    await userEvent.click(await screen.findByRole("option", { name: "Has location" }));
    expect(onChange).toHaveBeenCalledWith({ hasGps: true });

    fireEvent.click(screen.getByLabelText("Rate 3 stars"));
    expect(onChange).toHaveBeenCalledWith({ minRating: 3 });
  });

  it("types a camera", async () => {
    const onChange = mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    fireEvent.change(await screen.findByLabelText("Camera"), { target: { value: "ILCE-7M4" } });
    expect(onChange).toHaveBeenCalledWith({ camera: "ILCE-7M4" });
  });

  it("offers Has text only while reading text is on (Round 13-3)", async () => {
    mockCommand("get_settings", () => ({ appearance: { reduceMotion: false }, smart: { enabled: true, experimental: { readText: true } } }));
    const onChange = mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    fireEvent.click(await screen.findByLabelText("Has text"));
    expect(onChange).toHaveBeenCalledWith({ hasText: true });
  });

  it("leaves Has text out while reading text is off", async () => {
    mockCommand("get_settings", () => ({ appearance: { reduceMotion: false }, smart: { enabled: true, experimental: { readText: false } } }));
    mount();
    await userEvent.click(screen.getByRole("button", { name: /Filters/ }));
    expect(await screen.findByRole("combobox", { name: "Kind" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Has text")).toBeNull();
  });
});
