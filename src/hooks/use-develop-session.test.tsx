import { act, renderHook, waitFor } from "@testing-library/react";
import { useQueryClient } from "@tanstack/react-query";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import { FLUSH_REQUESTED, invalidateLibraryData } from "@/lib/events";
import { queryKeys } from "@/lib/query-keys";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { IDENTITY } from "@/hooks/use-edit-session";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults, windowMock } from "@/test/tauri-mock";
import type { DevelopPreview } from "@/types/generated/DevelopPreview";
import { useDevelopSession } from "./use-develop-session";

const IDENTITY_BASIC = DEFAULT_DEVELOP.basic;
const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

const rendered = (token: string): DevelopPreview => ({
  token,
  width: 300,
  height: 200,
  superseded: false,
  histogram: { r: [], g: [], b: [], luma: [], clippedShadows: 0, clippedHighlights: 0 },
  cameraRendering: false,
});

function mount(id = 7) {
  return renderHook(({ assetId }) => useDevelopSession(assetId), { wrapper: Providers, initialProps: { assetId: id } });
}

describe("useDevelopSession", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_develop_draft", () => null);
    mockCommand("develop_preview", () => rendered("1"));
    mockCommand("push_history", () => 1);
    mockCommand("truncate_history", () => 0);
    mockCommand("save_develop_draft", () => 1);
    mockCommand("hold_close", () => null);
  });

  it("renders the stored recipe once it is loaded", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.preview?.url).toContain("edit-preview/1"));
    expect(result.current.histogram).not.toBeNull();
    expect(calls("develop_preview")[0]).toMatchObject({ assetId: 7, recipe: IDENTITY, before: false, clipping: false });
  });

  it("numbers requests above any a page loaded earlier could have sent", async () => {
    // Rust keeps each photo's newest request number for the life of the process; a reloaded
    // page (Round 15's recovery) starting again at 1 would have every preview called stale.
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    const [first] = calls("develop_preview");
    expect(Number(first.request)).toBeGreaterThan(Date.now() - 60 * 60 * 1000);
  });

  it("keeps one render in flight and sends only the newest after it", async () => {
    const replies: Array<(p: DevelopPreview) => void> = [];
    mockCommand("develop_preview", () => new Promise<DevelopPreview>((r) => replies.push(r)));
    const { result } = mount();
    await waitFor(() => expect(replies).toHaveLength(1));
    act(() => result.current.update({ basic: { ...result.current.develop.basic, exposure: 0.1 } }));
    act(() => result.current.update({ basic: { ...result.current.develop.basic, exposure: 0.2 } }));
    act(() => result.current.update({ basic: { ...result.current.develop.basic, exposure: 0.3 } }));
    expect(replies).toHaveLength(1);
    await act(async () => replies[0](rendered("a")));
    await waitFor(() => expect(replies).toHaveLength(2));
    const sent = calls("develop_preview");
    expect((sent[1].recipe as typeof IDENTITY).develop?.basic.exposure).toBe(0.3);
    // Rust overtook it: nothing changes on screen.
    await act(async () => replies[1]({ ...rendered(""), superseded: true }));
    expect(result.current.preview?.url).toContain("edit-preview/a");
  });

  it("an AI change is one step that undoAi takes back, and only that step", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    const warmer = { ...IDENTITY, develop: { ...DEFAULT_DEVELOP, basic: { ...IDENTITY_BASIC, temperature: 12 } } };
    act(() => void expect(result.current.applyExternal(warmer, "AI: warmer")).toBe(true));
    expect(result.current.develop.basic.temperature).toBe(12);
    await waitFor(() => expect(calls("push_history").at(-1)).toMatchObject({ label: "AI: warmer" }));
    let back: ReturnType<typeof result.current.undoAi> = null;
    act(() => {
      back = result.current.undoAi();
    });
    expect(back).toEqual(IDENTITY);
    expect(result.current.develop.basic.temperature).toBe(0);
    // The newest step is now the user's.
    act(() => result.current.apply((d) => ({ ...d, basic: { ...d.basic, contrast: 0.2 } }), "Contrast"));
    act(() => {
      back = result.current.undoAi();
    });
    expect(back).toBeNull();
    expect(result.current.develop.basic.contrast).toBe(0.2);
  });

  it("Auto sends the draft on screen and applies its Basic, flagged auto, as one step", async () => {
    mockCommand("develop_auto", () => ({ basic: { ...IDENTITY_BASIC, exposure: 0.6 }, usedModel: true, scene: "sunset" }));
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, contrast: 0.2 } })));
    let r: Awaited<ReturnType<typeof result.current.auto>> = null;
    await act(async () => {
      r = await result.current.auto();
    });
    expect(r).toMatchObject({ scene: "sunset" });
    expect(calls("develop_auto")[0]).toMatchObject({ assetId: 7 });
    expect((calls("develop_auto")[0].recipe as typeof IDENTITY).develop?.basic.contrast).toBe(0.2);
    expect(result.current.develop.basic.exposure).toBe(0.6);
    expect(result.current.develop.auto).toBe(true);
    await waitFor(() => expect(calls("push_history").at(-1)).toMatchObject({ label: "Auto" }));
  });

  it("Auto with the trained model applies its curve, HSL and grading with Basic, in one step", async () => {
    const curve: typeof DEFAULT_DEVELOP.curve = { ...DEFAULT_DEVELOP.curve, master: [[0, 0], [0.5, 0.6], [1, 1]] };
    const look = { curve, hsl: DEFAULT_DEVELOP.hsl, grading: DEFAULT_DEVELOP.grading };
    mockCommand("develop_auto", () => ({ basic: { ...IDENTITY_BASIC, exposure: 0.4 }, usedModel: true, scene: null, source: "model", model: "r1", look }));
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => {
      await result.current.auto("t1");
    });
    expect(calls("develop_auto")[0]).toMatchObject({ task: "t1" });
    expect(result.current.develop.curve).toEqual(curve);
    expect(result.current.develop.basic.exposure).toBe(0.4);
    // Tone and colour only: the draft keeps its own detail.
    expect(result.current.develop.detail).toEqual(DEFAULT_DEVELOP.detail);
    await waitFor(() => expect(calls("push_history").filter((a) => a.label === "Auto")).toHaveLength(1));
  });

  it("Retouch applies develop, crop and straighten as one step, and one Undo brings the old crop back", async () => {
    const old = { x: 0.1, y: 0.1, w: 0.5, h: 0.5 };
    mockCommand("get_develop_draft", () => ({ ...IDENTITY, crop: old, straighten: 2 }));
    const develop = { ...DEFAULT_DEVELOP, basic: { ...IDENTITY_BASIC, exposure: 0.3 } };
    mockCommand("develop_retouch", () => ({ develop, crop: { x: 0, y: 0.05, w: 1, h: 0.9 }, straighten: -1.5, usedModel: true, summary: ["tone"], model: "r1" }));
    const { result } = mount();
    await waitFor(() => expect(result.current.draft.crop).toEqual(old));
    let r: Awaited<ReturnType<typeof result.current.retouch>> = null;
    await act(async () => {
      r = await result.current.retouch("t2");
    });
    expect(r).toMatchObject({ usedModel: true });
    expect(calls("develop_retouch")[0]).toMatchObject({ assetId: 7, task: "t2" });
    expect(result.current.draft).toMatchObject({ crop: { x: 0, y: 0.05, w: 1, h: 0.9 }, straighten: -1.5 });
    expect(result.current.develop.basic.exposure).toBe(0.3);
    act(() => result.current.undo());
    expect(result.current.draft.crop).toEqual(old);
    expect(result.current.draft.straighten).toBe(2);
    expect(result.current.develop.basic.exposure).toBe(IDENTITY_BASIC.exposure);
  });

  it("a Retouch no longer wanted (cancelled) changes nothing and records nothing", async () => {
    mockCommand("develop_retouch", () => ({ develop: DEFAULT_DEVELOP, crop: { x: 0, y: 0, w: 0.5, h: 0.5 }, straighten: 3, usedModel: true, summary: [], model: null }));
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    let r: Awaited<ReturnType<typeof result.current.retouch>> = null;
    await act(async () => {
      r = await result.current.retouch("t3", () => false);
    });
    expect(r).toBeNull();
    expect(result.current.draft.crop).toBeNull();
    expect(result.current.canUndo).toBe(false);
  });

  it("renders again on refresh, with nothing changed (a rebuilt denoise)", async () => {
    const { result } = mount();
    await waitFor(() => expect(calls("develop_preview")).toHaveLength(1));
    act(() => result.current.refresh());
    await waitFor(() => expect(calls("develop_preview")).toHaveLength(2));
    expect(result.current.assetId).toBe(7);
  });

  // Round 6 bug: the saved recipe was cached for good, and coming back to a photo began
  // from what it had been when first opened. Every change now goes into the cache at once.
  it("comes back to a photo with its edits, not as it was when first opened", async () => {
    const { result, rerender } = mount(7);
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.4 } })));
    act(() => result.current.commit("Exposure +0.40"));
    rerender({ assetId: 8 });
    await waitFor(() => expect(result.current.assetId).toBe(8));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    rerender({ assetId: 7 });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.develop.basic.exposure).toBe(0.4);
  });

  it("records a drag once, as one history step and one save", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    for (const v of [0.1, 0.2, 0.3]) act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: v } })));
    act(() => result.current.commit("Exposure +0.30"));
    await waitFor(() => expect(calls("push_history")).toHaveLength(1));
    expect(calls("push_history")[0]).toMatchObject({ assetId: 7, label: "Exposure +0.30" });
    await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 2000 });
    expect((calls("save_develop_draft")[0].recipe as typeof IDENTITY).develop?.basic.exposure).toBe(0.3);
    // Round 6: into Develop's draft; the library's recipe is never written from Develop.
    expect(calls("save_edit")).toHaveLength(0);
    act(() => result.current.commit("Exposure +0.30"));
    expect(calls("push_history")).toHaveLength(1);
  });

  it("undoes to the previous recorded state and redoes", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.apply((d) => ({ ...d, basic: { ...d.basic, contrast: 0.5 } }), "Contrast +50"));
    act(() => result.current.apply((d) => ({ ...d, basic: { ...d.basic, contrast: 0.8 } }), "Contrast +80"));
    act(() => result.current.undo());
    expect(result.current.develop.basic.contrast).toBe(0.5);
    act(() => result.current.undo());
    expect(result.current.draft.develop).toBeNull();
    expect(result.current.canUndo).toBe(false);
    act(() => result.current.redo());
    expect(result.current.develop.basic.contrast).toBe(0.5);
  });

  it("goes back to a history step and drops the newer ones on the next change", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.goTo({ ...IDENTITY, rotate: 90 }, 12));
    expect(result.current.draft.rotate).toBe(90);
    expect(calls("truncate_history")).toHaveLength(0);
    act(() => result.current.apply((d) => ({ ...d, basic: { ...d.basic, vibrance: 0.2 } }), "Vibrance +20"));
    await waitFor(() => expect(calls("push_history")).toHaveLength(1));
    expect(calls("truncate_history")[0]).toEqual({ assetId: 7, stepId: 12 });
    const truncateAt = invokeMock.mock.calls.findIndex(([c]) => c === "truncate_history");
    const pushAt = invokeMock.mock.calls.findIndex(([c]) => c === "push_history");
    expect(truncateAt).toBeLessThan(pushAt);
  });

  it("previews a hovered preset without changing the draft", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.setHover({ ...IDENTITY, rotate: 180 }));
    await waitFor(() => expect(calls("develop_preview").some((a) => (a.recipe as typeof IDENTITY).rotate === 180)).toBe(true));
    expect(result.current.draft.rotate).toBe(0);
  });

  it("keeps the last photo on screen until the next one's preview arrives", async () => {
    const replies: Array<(p: DevelopPreview) => void> = [];
    const { result, rerender } = mount(7);
    await waitFor(() => expect(result.current.preview?.assetId).toBe(7));
    mockCommand("develop_preview", () => new Promise<DevelopPreview>((r) => replies.push(r)));
    rerender({ assetId: 8 });
    // Photo 7 stays, with its histogram, while photo 8 renders.
    expect(result.current.preview?.assetId).toBe(7);
    expect(result.current.histogram).not.toBeNull();
    await waitFor(() => expect(replies).toHaveLength(1));
    await act(async () => replies[0](rendered("8")));
    await waitFor(() => expect(result.current.preview?.assetId).toBe(8));
  });

  it("never shows a late render of a photo already left behind", async () => {
    const replies: Array<(p: DevelopPreview) => void> = [];
    mockCommand("develop_preview", () => new Promise<DevelopPreview>((r) => replies.push(r)));
    const { result, rerender } = mount(7);
    await waitFor(() => expect(replies).toHaveLength(1));
    rerender({ assetId: 8 });
    await act(async () => replies[0](rendered("late-7")));
    await waitFor(() => expect(replies).toHaveLength(2));
    expect(result.current.preview).toBeNull();
    await act(async () => replies[1](rendered("8")));
    await waitFor(() => expect(result.current.preview?.url).toContain("edit-preview/8"));
  });

  it("ignores changes until the next photo's settings are in, so none land on the wrong photo", async () => {
    let answer: (r: null) => void = () => {};
    const { result, rerender } = mount(7);
    await waitFor(() => expect(result.current.loaded).toBe(true));
    mockCommand("get_develop_draft", () => new Promise<null>((r) => (answer = r)));
    rerender({ assetId: 8 });
    expect(result.current.loaded).toBe(false);
    act(() => result.current.apply({ basic: { ...result.current.develop.basic, exposure: 2 } }, "Exposure +2.00"));
    expect(calls("push_history")).toHaveLength(0);
    await act(async () => answer(null));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.develop.basic.exposure).toBe(0);
  });

  it("opens a photo with its settings as they are now, not as they were prefetched", async () => {
    const both = renderHook(({ assetId }) => ({ s: useDevelopSession(assetId), qc: useQueryClient() }), { wrapper: Providers, initialProps: { assetId: 7 } });
    await waitFor(() => expect(both.result.current.s.loaded).toBe(true));
    // Photo 8 was fetched ahead with no settings; then settings were pasted onto it.
    await act(async () => both.result.current.qc.prefetchQuery({ queryKey: queryKeys.edit(8), queryFn: () => null, staleTime: Infinity }));
    const pasted = { ...IDENTITY, develop: { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, exposure: 1.5 } } };
    mockCommand("get_develop_draft", (a) => (a?.assetId === 8 ? pasted : null));
    await act(async () => invalidateLibraryData(both.result.current.qc));
    both.rerender({ assetId: 8 });
    await waitFor(() => expect(both.result.current.s.loaded).toBe(true));
    expect(both.result.current.s.develop.basic.exposure).toBe(1.5);
  });

  it("undo while the next photo loads saves nothing onto it", async () => {
    let answer: (r: null) => void = () => {};
    const { result, rerender } = mount(7);
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.apply({ basic: { ...result.current.develop.basic, exposure: 1 } }, "Exposure +1.00"));
    await act(async () => result.current.flush());
    mockCommand("get_develop_draft", () => new Promise<null>((r) => (answer = r)));
    rerender({ assetId: 8 });
    act(() => result.current.undo());
    await act(async () => answer(null));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => result.current.flush());
    expect(calls("save_develop_draft").filter((c) => c.assetId === 8)).toHaveLength(0);
  });

  // Round 4J: before and after side by side.
  describe("comparing", () => {
    const cropped = { ...IDENTITY, crop: { x: 0.1, y: 0.2, w: 0.5, h: 0.5 }, develop: DEFAULT_DEVELOP };

    it("renders the before once per photo, not on every slider move", async () => {
      mockCommand("develop_before", () => rendered("before"));
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      expect(calls("develop_before")).toHaveLength(0);
      act(() => result.current.setCompare("split"));
      await waitFor(() => expect(result.current.beforeView?.url).toContain("edit-preview/before"));
      for (const e of [0.1, 0.2, 0.3, 0.4, 0.5]) act(() => result.current.update({ basic: { ...result.current.develop.basic, exposure: e } }));
      act(() => result.current.setCompare("side"));
      await waitFor(() => expect(calls("develop_preview").length).toBeGreaterThan(1));
      expect(calls("develop_before")).toHaveLength(1);
      // The main render stays the after in split and side by side.
      expect(calls("develop_preview").every((c) => c.before === false)).toBe(true);
    });

    it("keeps a cropped photo's geometry in its before", async () => {
      mockCommand("get_develop_draft", () => cropped);
      mockCommand("develop_before", () => rendered("before"));
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.setCompare("split"));
      await waitFor(() => expect(calls("develop_before")).toHaveLength(1));
      const sent = calls("develop_before")[0].recipe as typeof IDENTITY;
      expect(sent.crop).toEqual(cropped.crop);
      expect(sent.develop).toBeNull();
    });

    it("keeps `before` for the toggle, and setBefore for its callers", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.setBefore(true));
      expect(result.current.compare).toBe("toggle");
      expect(result.current.before).toBe(true);
      await waitFor(() => expect(calls("develop_preview").some((c) => c.before === true)).toBe(true));
      act(() => result.current.setCompare("split"));
      expect(result.current.before).toBe(false);
      act(() => result.current.setBefore(false));
      expect(result.current.compare).toBe("off");
    });
  });

  // Round 5: every change is saved, not only what a release records.
  describe("autosave", () => {
    const exposure = (r: unknown) => (r as typeof IDENTITY).develop?.basic.exposure;

    it("saves a change still being dragged, after a pause, once for a burst", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      for (const e of [0.1, 0.2, 0.3]) act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: e } })));
      expect(calls("save_develop_draft")).toHaveLength(0);
      await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 3000 });
      expect(exposure(calls("save_develop_draft")[0].recipe)).toBe(0.3);
      // A drag is not a history step.
      expect(calls("push_history")).toHaveLength(0);
    });

    it("holds the window open while a save waits and lets it go once written", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.4 } })));
      await waitFor(() => expect(calls("hold_close")).toEqual([{ hold: true }]));
      await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 3000 });
      await waitFor(() => expect(calls("hold_close").at(-1)).toEqual({ hold: false }));
    });

    it("writes a waiting save at once when the window asks before closing", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.7 } })));
      const closes = windowMock.close.mock.calls.length;
      await act(async () => emitMock(FLUSH_REQUESTED, true));
      await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 200 });
      expect(exposure(calls("save_develop_draft")[0].recipe)).toBe(0.7);
      await waitFor(() => expect(calls("hold_close").at(-1)).toEqual({ hold: false }));
      await waitFor(() => expect(windowMock.close.mock.calls.length).toBe(closes + 1));
    });

    it("writes a waiting save without closing when Restart Lumen asks", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.6 } })));
      const closes = windowMock.close.mock.calls.length;
      await act(async () => emitMock(FLUSH_REQUESTED, false));
      await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 200 });
      await waitFor(() => expect(calls("hold_close").at(-1)).toEqual({ hold: false }));
      expect(windowMock.close.mock.calls.length).toBe(closes);
    });

    it("keeps holding while a save is still being written when the window asks", async () => {
      let finish: (v: number) => void = () => {};
      mockCommand("save_develop_draft", () => new Promise<number>((r) => (finish = r)));
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.2 } })));
      await waitFor(() => expect(calls("save_develop_draft")).toHaveLength(1), { timeout: 3000 });
      // The save is in flight (a busy database); Restart asks for the edits now.
      await act(async () => emitMock(FLUSH_REQUESTED, false));
      await new Promise((r) => setTimeout(r, 50));
      expect(calls("hold_close").at(-1)).toEqual({ hold: true });
      await act(async () => finish(1));
      await waitFor(() => expect(calls("hold_close").at(-1)).toEqual({ hold: false }));
    });

    it("undo still returns to the last recorded step", async () => {
      const { result } = mount();
      await waitFor(() => expect(result.current.loaded).toBe(true));
      act(() => result.current.apply((d) => ({ ...d, basic: { ...d.basic, exposure: 0.5 } }), "Exposure"));
      act(() => result.current.update((d) => ({ ...d, basic: { ...d.basic, exposure: 0.9 } })));
      act(() => result.current.commit("Exposure"));
      act(() => result.current.undo());
      expect(result.current.develop.basic.exposure).toBe(0.5);
    });
  });
});
