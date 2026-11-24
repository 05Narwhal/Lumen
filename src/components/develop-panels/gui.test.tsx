import { MotionGlobalConfig } from "framer-motion";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Providers } from "@/app";
import { DENOISE_FINISHED, DENOISE_PROGRESS } from "@/lib/events";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { areaPath, linePath } from "@/components/develop-histogram";
import { DevelopSlider } from "@/components/develop-slider";
import { IDENTITY } from "@/hooks/use-edit-session";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP, developOf } from "@/lib/develop";
import { toastStore } from "@/components/toast/store";
import type { AutoResult } from "@/types/generated/AutoResult";
import type { RetouchResult } from "@/types/generated/RetouchResult";
import { queryKeys } from "@/lib/query-keys";
import { useQueryClient } from "@tanstack/react-query";
import type { Develop } from "@/types/generated/Develop";
import type { Recipe } from "@/types/generated/Recipe";
import { pointToHueSat, puckPosition } from "./color-wheel";
import { DevelopPanels } from "./gui";
import { requestRetouch } from "./retouch";

// The AI buttons crossfade their labels; an exit animation never finishes in jsdom.
MotionGlobalConfig.skipAnimations = true;

/** A session over a plain object: `update` changes it, `commit` and `apply` are spied. */
function fakeSession(develop: Develop = DEFAULT_DEVELOP) {
  let draft: Recipe = { ...IDENTITY, develop };
  const s = {
    get draft() {
      return draft;
    },
    get develop() {
      return developOf(draft);
    },
    histogram: null,
    clipping: false,
    setClipping: vi.fn(),
    assetId: 7,
    refresh: vi.fn(),
    update: vi.fn((c: Partial<Develop> | ((d: Develop) => Develop)) => {
      const d = developOf(draft);
      draft = { ...draft, develop: typeof c === "function" ? c(d) : { ...d, ...c } };
    }),
    commit: vi.fn(),
    apply: vi.fn((c: (d: Develop) => Develop) => {
      draft = { ...draft, develop: c(developOf(draft)) };
    }),
  };
  return s as unknown as DevelopSession & { update: ReturnType<typeof vi.fn>; commit: ReturnType<typeof vi.fn>; apply: ReturnType<typeof vi.fn>; refresh: ReturnType<typeof vi.fn> };
}

// The panels read Denoise's status through react-query, so they need the app's providers.
/** Settings → Experimental's rows as far as Develop reads them: whether a Prism model is installed. */
const catalog = (prism: boolean) => [{ task: "retouch", family: "prism", models: prism ? [{ id: "lumen-prism-1.0.1-d", name: "Lumen Prism 1.0.1-d", version: "1.0.1-d", legacy: false, tiers: [] }] : [] }];

const wrap = (ui: React.ReactNode, settings: unknown = null, prism = true) => {
  mockLibraryDefaults();
  mockCommand("get_settings", () => settings);
  mockCommand("experimental_catalog", () => catalog(prism));
  mockCommand("denoise_status", () => ({ available: true, reason: null, model: "m@1", ready: false, running: false, gpu: true }));
  return render(<Providers>{ui}</Providers>);
};

describe("DevelopSlider", () => {
  it("records a key press once, named with its value, and resets on double-click", async () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    wrap(<DevelopSlider label="Contrast" value={20} min={-100} max={100} format={(v) => `+${v}`} onChange={onChange} onCommit={onCommit} />);
    const thumb = screen.getByRole("slider", { name: "Contrast" });
    thumb.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenLastCalledWith(21, "Contrast +21");
    fireEvent.doubleClick(thumb);
    expect(onChange).toHaveBeenLastCalledWith(0);
    expect(onCommit).toHaveBeenLastCalledWith(0, "Contrast reset");
  });
});

describe("the colour wheel", () => {
  it("maps a point to hue and strength, and back", () => {
    expect(pointToHueSat(0, -40, 40)).toEqual({ hue: 90, sat: 1 });
    expect(pointToHueSat(20, 0, 40)).toEqual({ hue: 0, sat: 0.5 });
    expect(pointToHueSat(-80, 0, 40)).toEqual({ hue: 180, sat: 1 });
    const p = puckPosition(90, 1);
    expect(p.left).toBeCloseTo(0.5);
    expect(p.top).toBeCloseTo(0);
  });
});

describe("the histogram", () => {
  it("draws an area and a line across all bins", () => {
    const counts = Array.from({ length: 256 }, (_, i) => (i === 128 ? 100 : 1));
    const area = areaPath(counts, 100);
    expect(area.startsWith("M0 100")).toBe(true);
    expect(area).toContain("L128 0.00");
    expect(area.endsWith("Z")).toBe(true);
    expect(linePath(counts, 100).split("L")).toHaveLength(256);
    expect(areaPath([], 1)).toBe("");
  });
});

describe("Denoise in Detail", () => {
  const status = (over: object = {}) => ({ available: true, reason: null, model: "m@1", ready: false, running: false, gpu: true, ...over });
  const withDenoise = { ...DEFAULT_DEVELOP, detail: { ...DEFAULT_DEVELOP.detail, denoise: { strength: 0.5, model: "m@1" } } };
  const mount = (session: ReturnType<typeof fakeSession>, st: object = status()) => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("denoise_status", () => st);
    mockCommand("start_denoise", () => null);
    mockCommand("cancel_denoise", () => null);
    render(
      <Providers>
        <DevelopPanels session={session} />
      </Providers>,
    );
  };

  it("runs, shows progress and adds the setting when it finishes", async () => {
    const session = fakeSession();
    mount(session);
    await userEvent.click(await screen.findByRole("button", { name: "Denoise" }));
    expect(invokeMock).toHaveBeenCalledWith("start_denoise", { assetId: 7 });
    act(() => emitMock(DENOISE_PROGRESS, { assetId: 7, done: 2, total: 8 }));
    // The button is the progress: its ring takes the fraction done.
    const button = screen.getByRole("button", { name: "Denoising" });
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button.style.getPropertyValue("--ai-progress")).toBe("0.25");
    act(() => emitMock(DENOISE_FINISHED, { assetId: 7, ok: true, model: "m@1", onGpu: true, error: null }));
    expect(session.apply).toHaveBeenCalledWith(expect.any(Function), "Denoise");
    expect(session.develop.detail.denoise).toEqual({ strength: 0.5, model: "m@1" });
  });

  it("is disabled with the reason when unavailable", async () => {
    mount(fakeSession(), status({ available: false, reason: "Turn on smart features to denoise." }));
    expect(await screen.findByText("Turn on smart features to denoise.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Denoise" })).toBeDisabled();
  });

  it("cancels from the button and goes back to Denoise", async () => {
    mount(fakeSession());
    await userEvent.click(await screen.findByRole("button", { name: "Denoise" }));
    await userEvent.click(screen.getByRole("button", { name: "Denoising" }));
    expect(invokeMock).toHaveBeenCalledWith("cancel_denoise", { assetId: 7 });
    expect(screen.queryByRole("button", { name: "Denoising" })).toBeNull();
    expect(screen.getByRole("button", { name: "Denoise" })).toBeEnabled();
  });

  it("with denoise: a strength slider named for history, and Remove", async () => {
    const session = fakeSession(withDenoise);
    mount(session, status({ ready: true }));
    const slider = await screen.findByRole("slider", { name: "Denoise strength" });
    slider.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(session.commit).toHaveBeenCalledWith("Denoise strength 51");
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(session.apply).toHaveBeenLastCalledWith(expect.any(Function), "Remove denoise");
    expect(session.develop.detail.denoise).toBeNull();
  });

  it("a recipe with denoise but no base offers Rebuild, and re-renders when it is back", async () => {
    const session = fakeSession(withDenoise);
    mount(session, status({ ready: false }));
    expect(await screen.findByText("Denoise needs to be rebuilt for this photo.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Rebuild" }));
    expect(invokeMock).toHaveBeenCalledWith("start_denoise", { assetId: 7 });
    act(() => emitMock(DENOISE_FINISHED, { assetId: 7, ok: true, model: "m@1", onGpu: true, error: null }));
    expect(session.refresh).toHaveBeenCalled();
    expect(session.apply).not.toHaveBeenCalled();
  });
});

const CLASSIC: AutoResult = { basic: DEFAULT_DEVELOP.basic, usedModel: true, scene: null, source: "classic", model: null, look: null };
const RETOUCHED: RetouchResult = { develop: DEFAULT_DEVELOP, crop: null, straighten: 0, usedModel: true, summary: ["tone"], model: "Lumen Ray 1.0" };
const experimental = (retouch: boolean, enabled = true) => ({
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  smart: { enabled, experimental: { autoModel: false, retouch } },
});

describe("DevelopPanels", () => {
  it("a second click cancels Auto: the backend is told, and nothing is applied", async () => {
    toastStore.set({ toasts: [] });
    mockCommand("ai_cancel", () => true);
    let finish: (r: AutoResult | null) => void = () => {};
    let wanted: () => boolean = () => true;
    const session = Object.assign(fakeSession(), {
      auto: vi.fn((_task: string, stillWanted: () => boolean) => {
        wanted = stillWanted;
        return new Promise<AutoResult | null>((r) => (finish = r));
      }),
    });
    wrap(<DevelopPanels session={session} />);
    const button = screen.getByRole("button", { name: "Auto" });
    await userEvent.click(button);
    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "true"));
    await userEvent.click(button);
    expect(invokeMock).toHaveBeenCalledWith("ai_cancel", expect.objectContaining({ task: expect.any(String) }));
    expect(wanted()).toBe(false);
    await act(async () => finish(null));
    expect(button).not.toHaveAttribute("data-busy");
    expect(toastStore.get().toasts).toHaveLength(0);
  });

  it("has no Retouch with the switch off, and shows it once the setting changes, without remounting", async () => {
    let qc: ReturnType<typeof useQueryClient> | null = null;
    const Grab = () => {
      qc = useQueryClient();
      return null;
    };
    wrap(
      <>
        <Grab />
        <DevelopPanels session={fakeSession()} />
      </>,
      experimental(false),
    );
    await screen.findByRole("button", { name: "Auto" });
    expect(screen.queryByRole("button", { name: "Retouch" })).toBeNull();
    act(() => qc!.setQueryData(queryKeys.settings, experimental(true)));
    expect(await screen.findByRole("button", { name: "Retouch" })).toBeInTheDocument();
    // Smart features off: the backend would only run classic Auto, so there is no Retouch.
    act(() => qc!.setQueryData(queryKeys.settings, experimental(true, false)));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Retouch" })).toBeNull());
  });

  it("Retouch runs once at a time and says what it changed", async () => {
    toastStore.set({ toasts: [] });
    let finish: (r: RetouchResult) => void = () => {};
    const session = Object.assign(fakeSession(), { retouch: vi.fn(() => new Promise<RetouchResult>((r) => (finish = r))) });
    wrap(<DevelopPanels session={session} />, experimental(true));
    const button = await screen.findByRole("button", { name: "Retouch" });
    await userEvent.click(button);
    // A key press while it runs is not a second run.
    act(() => requestRetouch());
    expect(session.retouch).toHaveBeenCalledTimes(1);
    await act(async () => finish(RETOUCHED));
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Retouch with Lumen Ray 1.0 — tone");
  });

  it("the Retouch key does nothing while the switch is off", async () => {
    const session = Object.assign(fakeSession(), { retouch: vi.fn(async () => RETOUCHED) });
    wrap(<DevelopPanels session={session} />, experimental(false));
    await screen.findByRole("button", { name: "Auto" });
    act(() => requestRetouch());
    await new Promise((r) => setTimeout(r, 50));
    expect(session.retouch).not.toHaveBeenCalled();
  });

  // Round 14: Retouch runs Lumen Prism; until one is installed it waits, In development.
  it("Retouch is In development without a Prism model: disabled, and its key does nothing", async () => {
    const session = Object.assign(fakeSession(), { retouch: vi.fn(async () => RETOUCHED) });
    wrap(<DevelopPanels session={session} />, experimental(true), false);
    const button = await screen.findByRole("button", { name: "Retouch" });
    await waitFor(() => expect(button).toBeDisabled());
    expect(screen.getByText("In development")).toBeInTheDocument();
    expect(button).toHaveAttribute("title", "Retouch arrives with Lumen Prism.");
    act(() => requestRetouch());
    await new Promise((r) => setTimeout(r, 50));
    expect(session.retouch).not.toHaveBeenCalled();
  });

  it("with a Prism model the Retouch key runs it", async () => {
    const session = Object.assign(fakeSession(), { retouch: vi.fn(async () => RETOUCHED) });
    wrap(<DevelopPanels session={session} />, experimental(true), true);
    const button = await screen.findByRole("button", { name: "Retouch" });
    await waitFor(() => expect(button).toBeEnabled());
    expect(screen.queryByText("In development")).toBeNull();
    act(() => requestRetouch());
    await waitFor(() => expect(session.retouch).toHaveBeenCalledTimes(1));
  });

  it("Auto runs once at a time and says what it read the photo as", async () => {
    toastStore.set({ toasts: [] });
    let finish: (r: AutoResult) => void = () => {};
    const session = Object.assign(fakeSession(), { auto: vi.fn(() => new Promise<AutoResult>((r) => (finish = r))) });
    wrap(<DevelopPanels session={session} />);
    const button = screen.getByRole("button", { name: "Auto" });
    await userEvent.click(button);
    // Inside the delay it still looks idle: nothing flashes for a fast Auto.
    expect(button).not.toHaveAttribute("data-busy");
    expect(button).toHaveTextContent("Auto");
    await waitFor(() => expect(button).toHaveAttribute("aria-busy", "true"));
    await waitFor(() => expect(button).toHaveTextContent("Working…"));
    expect(button).toHaveAttribute("data-indeterminate");
    expect(session.auto).toHaveBeenCalledTimes(1);
    await act(async () => finish({ ...CLASSIC, scene: "sunset" }));
    expect(button).not.toHaveAttribute("data-busy");
    expect(button).toHaveTextContent("Auto");
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Auto — read as a sunset");
  });

  it("a fast Auto never shows Working…", async () => {
    toastStore.set({ toasts: [] });
    const session = Object.assign(fakeSession(), { auto: vi.fn(async () => CLASSIC) });
    wrap(<DevelopPanels session={session} />);
    const button = screen.getByRole("button", { name: "Auto" });
    const seen: string[] = [];
    const watch = new MutationObserver(() => seen.push(button.getAttribute("data-busy") ?? "idle"));
    watch.observe(button, { attributes: true, subtree: true, characterData: true, childList: true });
    await userEvent.click(button);
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Auto"));
    await new Promise((r) => setTimeout(r, 300));
    watch.disconnect();
    expect(seen).not.toContain("");
    expect(button).toHaveTextContent("Auto");
    expect(button).not.toHaveTextContent("Working");
  });

  it("without a model it says Auto only measured the photo", async () => {
    toastStore.set({ toasts: [] });
    const session = Object.assign(fakeSession(), { auto: vi.fn(async () => ({ ...CLASSIC, usedModel: false })) });
    wrap(<DevelopPanels session={session} />);
    await userEvent.click(screen.getByRole("button", { name: "Auto" }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Auto measured the photo; smart features are off or still downloading."));
  });

  it("switches the HSL bands between hue, saturation and luminance", async () => {
    const hsl = DEFAULT_DEVELOP.hsl.map((b, i) => (i === 0 ? { h: 0.2, s: -0.5, l: 0 } : b)) as Develop["hsl"];
    wrap(<DevelopPanels session={fakeSession({ ...DEFAULT_DEVELOP, hsl })} />);
    expect(screen.getByRole("slider", { name: "Red hue" })).toHaveAttribute("aria-valuenow", "20");
    await userEvent.click(screen.getByRole("radio", { name: "Saturation" }));
    expect(screen.getByRole("slider", { name: "Red saturation" })).toHaveAttribute("aria-valuenow", "-50");
  });

  it("resets one panel and leaves the others", async () => {
    const session = fakeSession({ ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, exposure: 1 }, effects: { ...DEFAULT_DEVELOP.effects, grain: { amount: 0.3, size: 0.25 } } });
    wrap(<DevelopPanels session={session} />);
    await userEvent.click(screen.getByRole("button", { name: "Reset Basic" }));
    expect(session.apply).toHaveBeenCalledWith(expect.any(Function), "Reset Basic");
    expect(session.develop.basic.exposure).toBe(0);
    expect(session.develop.effects.grain.amount).toBe(0.3);
    expect(screen.getByRole("button", { name: "Reset Detail" })).toBeDisabled();
  });

  it("sets white balance back to as shot", async () => {
    const session = fakeSession({ ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, temperature: 30, exposure: 0.5 } });
    wrap(<DevelopPanels session={session} />);
    await userEvent.click(screen.getByRole("button", { name: "As shot" }));
    expect(session.develop.basic.temperature).toBe(0);
    expect(session.develop.basic.exposure).toBe(0.5);
  });

  it("turns a colour-grading wheel with the keyboard, recording each step", async () => {
    const session = fakeSession();
    wrap(<DevelopPanels session={session} />);
    const wheel = screen.getByRole("slider", { name: "Shadows colour" });
    wheel.focus();
    await userEvent.keyboard("{ArrowUp}");
    expect(session.develop.grading.shadows.sat).toBe(0.05);
    expect(session.commit).toHaveBeenLastCalledWith("Shadows colour 0° 5");
  });
});
