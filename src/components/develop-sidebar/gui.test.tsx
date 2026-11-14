import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { IDENTITY } from "@/hooks/use-edit-session";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP, developOf } from "@/lib/develop";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { Develop } from "@/types/generated/Develop";
import type { Recipe } from "@/types/generated/Recipe";
import { DevelopSidebar } from "./gui";

const vivid = { id: -2, key: "builtin:2", source: "builtin", name: "Vivid", develop: { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, vibrance: 0.4 } }, panels: ["basic"], builtin: true };
const mine = { id: 5, key: "db:5", source: "mine", name: "Moody", develop: DEFAULT_DEVELOP, panels: ["effects"], builtin: false };

function fakeSession() {
  let draft: Recipe = IDENTITY;
  return {
    get draft() {
      return draft;
    },
    get develop() {
      return developOf(draft);
    },
    setHover: vi.fn(),
    apply: vi.fn((c: (d: Develop) => Develop) => {
      draft = { ...draft, develop: c(developOf(draft)) };
    }),
    goTo: vi.fn(),
  } as unknown as DevelopSession & { setHover: ReturnType<typeof vi.fn>; apply: ReturnType<typeof vi.fn>; goTo: ReturnType<typeof vi.fn> };
}

function mount(session = fakeSession()) {
  render(
    <Providers>
      <DevelopSidebar session={session} assetId={3} />
    </Providers>,
  );
  return session;
}

describe("DevelopSidebar", () => {
  beforeEach(() => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_presets", () => [vivid, mine]);
    mockCommand("save_preset", (a) => ({ id: 9, key: "db:9", source: "mine", name: a?.name, develop: a?.develop, panels: a?.panels, builtin: false }));
    mockCommand("list_history", () => [
      { id: 1, label: "Exposure +0.30", recipe: { ...IDENTITY, rotate: 90 }, snapshotName: null, createdAt: 0 },
      { id: 2, label: "Contrast +20", recipe: { ...IDENTITY, rotate: 180 }, snapshotName: null, createdAt: 1 },
      { id: 3, label: "Before", recipe: IDENTITY, snapshotName: "Before", createdAt: 2 },
    ]);
  });

  it("previews a hovered preset without applying it, and applies it on click", async () => {
    const session = mount();
    const vividRow = await screen.findByRole("button", { name: "Vivid" });
    fireEvent.mouseEnter(vividRow);
    expect(session.setHover).toHaveBeenLastCalledWith(expect.objectContaining({ develop: expect.objectContaining({ basic: expect.objectContaining({ vibrance: 0.4 }) }) }));
    expect(session.apply).not.toHaveBeenCalled();
    fireEvent.mouseLeave(vividRow);
    expect(session.setHover).toHaveBeenLastCalledWith(null);
    await userEvent.click(vividRow);
    expect(session.apply).toHaveBeenCalledWith(expect.any(Function), "Preset: Vivid");
    expect(session.develop.basic.vibrance).toBe(0.4);
  });

  it("saves a preset with the chosen panels", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Save preset…" }));
    await userEvent.type(await screen.findByLabelText("Name"), "Harbour");
    await userEvent.click(screen.getByRole("checkbox", { name: "Effects" }));
    await userEvent.click(screen.getByRole("button", { name: "Save preset" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("save_preset", expect.objectContaining({ name: "Harbour", panels: ["basic", "curve", "hsl", "grading", "detail"] })));
  });

  it("lists history newest first and goes back to a step", async () => {
    const session = mount();
    const steps = await screen.findAllByRole("button", { name: /Exposure|Contrast/ });
    expect(steps.map((b) => b.textContent)).toEqual(["Contrast +20", "Exposure +0.30"]);
    await userEvent.click(screen.getByRole("button", { name: "Exposure +0.30" }));
    expect(session.goTo).toHaveBeenCalledWith({ ...IDENTITY, rotate: 90 }, 1);
  });

  it("keeps snapshots apart and applies one without a step", async () => {
    const session = mount();
    const snap = await screen.findByRole("button", { name: "Before" });
    expect(snap).toHaveAttribute("aria-current", "true");
    await userEvent.click(snap);
    expect(session.goTo).toHaveBeenCalledWith(IDENTITY, null);
  });
});
