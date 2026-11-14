import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { PersonSummary } from "@/types/generated/PersonSummary";
import { PersonPicker, ROW_HEIGHT } from "./gui";

const person = (id: number, name: string | null, photoCount = 10): PersonSummary => ({ id, name, faceCount: photoCount, photoCount, coverFaceId: id * 10 });

function mount(people: PersonSummary[], exclude = 0) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  const onPick = vi.fn();
  render(
    <Providers>
      <PersonPicker people={people} exclude={exclude} onPick={onPick} onClose={vi.fn()} />
    </Providers>,
  );
  return onPick;
}

const rows = () => within(screen.getByRole("listbox")).queryAllByRole("option");

describe("PersonPicker", () => {
  it("shows five rows at a time, named people first, then by photo count", async () => {
    const many = Array.from({ length: 30 }, (_, i) => person(i + 1, i % 3 === 0 ? null : `Person ${i + 1}`, i));
    mount(many);
    const list = await screen.findByRole("listbox");
    expect(list.style.maxHeight).toBe(`${5 * ROW_HEIGHT}px`);
    const shown = rows().map((r) => r.textContent ?? "");
    expect(shown.findIndex((t) => t.startsWith("Unnamed"))).toBe(20);
    expect(shown[20]).toContain("27 photos");
  });

  it("filters as you type, ignoring case and accents, and never offers the person themself", async () => {
    mount([person(1, "Ána"), person(2, "Jan"), person(3, "Ben"), person(4, "Anna Marie")], 4);
    const search = await screen.findByRole("textbox", { name: "Search people" });
    expect(search).toHaveFocus();
    await userEvent.type(search, "an");
    expect(rows().map((r) => r.textContent)).toEqual([expect.stringContaining("Ána"), expect.stringContaining("Jan")]);
  });

  it("moves with the arrow keys and picks with Enter", async () => {
    const onPick = mount([person(1, "Anna"), person(2, "Ben"), person(3, "Cleo")]);
    await screen.findByRole("listbox");
    await userEvent.keyboard("{ArrowDown}{ArrowDown}{ArrowUp}");
    expect(rows()[1]).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{Enter}");
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 2 }));
  });

  it("says when nobody matches", async () => {
    mount([person(1, "Anna")]);
    await userEvent.type(await screen.findByRole("textbox", { name: "Search people" }), "zo");
    expect(screen.getByText('No one called "zo"')).toBeInTheDocument();
  });

  it("makes a new person from what was typed when tagging (Round 4I)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const onCreate = vi.fn();
    const onPick = vi.fn();
    render(
      <Providers>
        <PersonPicker people={[person(1, "Anna")]} onPick={onPick} onCreate={onCreate} onClose={vi.fn()} title="Who is this?" />
      </Providers>,
    );
    expect(await screen.findByRole("option", { name: /New person…/ })).toBeInTheDocument();
    const search = screen.getByRole("textbox", { name: "Search people" });
    await userEvent.type(search, "Zoe{Enter}");
    expect(onCreate).toHaveBeenCalledWith("Zoe");
    expect(onPick).not.toHaveBeenCalled();
  });

  it("asks for a name when New person… is chosen with nothing typed", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const onCreate = vi.fn();
    render(
      <Providers>
        <PersonPicker people={[]} onPick={vi.fn()} onCreate={onCreate} onClose={vi.fn()} />
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("option", { name: /New person…/ }));
    expect(onCreate).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox", { name: "Search people" })).toHaveAttribute("placeholder", "Type the new person's name");
  });
});
