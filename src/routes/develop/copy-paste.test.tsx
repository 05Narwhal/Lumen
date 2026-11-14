import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { IDENTITY } from "@/hooks/use-edit-session";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP, developClipboard, developSelection, developOf, ZERO_ADJUST } from "@/lib/develop";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { Develop } from "@/types/generated/Develop";
import { useCopyPaste } from "./copy-paste";

function fakeSession(develop: Develop = DEFAULT_DEVELOP) {
  let draft = { ...IDENTITY, develop };
  return {
    get draft() {
      return draft;
    },
    get develop() {
      return developOf(draft);
    },
    apply: vi.fn((c: (d: Develop) => Develop) => {
      draft = { ...draft, develop: c(developOf(draft)) };
    }),
    undo: vi.fn(),
  } as unknown as DevelopSession & { apply: ReturnType<typeof vi.fn>; undo: ReturnType<typeof vi.fn> };
}

const warm = { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, temperature: 30 } };

describe("copy and paste settings", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("paste_develop", (a) => (a?.ids as number[]).map((id) => ({ assetId: id, recipe: null })));
    mockCommand("restore_develop_drafts", () => null);
    developSelection.set({ ids: new Set(), anchor: null });
    developClipboard.set(null);
  });

  it("pastes onto the open photo when nothing is selected", async () => {
    developClipboard.set({ develop: warm, panels: ["basic"] });
    const session = fakeSession();
    const { result } = renderHook(() => useCopyPaste(session, 7), { wrapper: Providers });
    await act(() => result.current.paste());
    expect(session.apply).toHaveBeenCalledWith(expect.any(Function), "Pasted settings");
    expect(session.develop.basic.temperature).toBe(30);
    expect(invokeMock.mock.calls.some(([c]) => c === "paste_develop")).toBe(false);
  });

  it("pastes onto the selection, and one Undo takes it all back", async () => {
    developClipboard.set({ develop: warm, panels: ["basic"] });
    developSelection.set({ ids: new Set([7, 8, 9]), anchor: 7 });
    const session = fakeSession();
    const { result } = renderHook(() => useCopyPaste(session, 7), { wrapper: Providers });
    await act(() => result.current.paste());
    expect(invokeMock).toHaveBeenCalledWith("paste_develop", { ids: [8, 9], develop: warm, panels: ["basic"], label: "Pasted settings" });
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("Pasted settings onto 3 photos");
    act(() => t.action?.run());
    expect(session.undo).toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith("restore_develop_drafts", { previous: [{ assetId: 8, recipe: null }, { assetId: 9, recipe: null }] });
  });

  const masked = (id: number, exposure: number) => ({
    ...DEFAULT_DEVELOP,
    masks: [{ id, name: `Mask ${id}`, shapes: [], invert: true, opacity: 1, adjust: { ...ZERO_ADJUST, exposure }, visible: true }],
  });

  it("leaves the target's masks alone when Masks was not copied", async () => {
    developClipboard.set({ develop: { ...masked(1, 1), basic: warm.basic }, panels: ["basic"] });
    const session = fakeSession(masked(9, -1));
    const { result } = renderHook(() => useCopyPaste(session, 7), { wrapper: Providers });
    await act(() => result.current.paste());
    expect(session.develop.basic.temperature).toBe(30);
    expect(session.develop.masks?.map((m) => m.id)).toEqual([9]);
  });

  it("replaces the target's masks when Masks was copied", async () => {
    developClipboard.set({ develop: masked(1, 1), panels: ["masks"] });
    const session = fakeSession(masked(9, -1));
    const { result } = renderHook(() => useCopyPaste(session, 7), { wrapper: Providers });
    await act(() => result.current.paste());
    expect(session.develop.masks?.map((m) => m.id)).toEqual([1]);
  });

  it("offers Masks when copying, unticked, and never when saving a preset", async () => {
    const { PanelChoice } = await import("@/components/panel-choice");
    const onConfirm = vi.fn();
    const { rerender } = render(
      <Providers>
        <PanelChoice opened title="Copy settings" confirmLabel="Copy" masks onConfirm={onConfirm} onClose={() => {}} />
      </Providers>,
    );
    const box = await screen.findByRole("checkbox", { name: "Masks" });
    expect(box).not.toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onConfirm.mock.calls[0][0].panels).not.toContain("masks");
    await userEvent.click(box);
    await userEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onConfirm.mock.calls[1][0].panels).toContain("masks");
    rerender(
      <Providers>
        <PanelChoice opened title="Save preset" confirmLabel="Save preset" name="" onConfirm={onConfirm} onClose={() => {}} />
      </Providers>,
    );
    expect(screen.queryByRole("checkbox", { name: "Masks" })).toBeNull();
  });

  it("asks for a copy before a paste", async () => {
    const session = fakeSession();
    const { result } = renderHook(() => useCopyPaste(session, 7), { wrapper: Providers });
    await act(() => result.current.paste());
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Copy settings from a photo first.");
    expect(session.apply).not.toHaveBeenCalled();
  });
});
