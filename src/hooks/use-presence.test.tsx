import { act, renderHook } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { slideshowPlayingStore } from "@/lib/presence";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { usePresence } from "./use-presence";

const calls = () => invokeMock.mock.calls.filter(([cmd]) => cmd === "discord_set_screen").map(([, args]) => args);

function setup(enabled: boolean, at = "/library") {
  const wrapper = ({ children }: { children: ReactNode }) => <MemoryRouter initialEntries={[at]}>{children}</MemoryRouter>;
  return renderHook(
    ({ on }) => {
      usePresence(on);
      return useNavigate();
    },
    { wrapper, initialProps: { on: enabled } },
  );
}

describe("usePresence", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockClear();
    mockCommand("discord_set_screen", () => null);
    slideshowPlayingStore.set(false);
  });
  afterEach(() => vi.useRealTimers());

  it("sends the screen once on mount, as one key and nothing else", () => {
    setup(true);
    expect(calls()).toEqual([{ screen: "browsing" }]);
  });

  it("sends nothing while off", () => {
    const { result } = setup(false);
    act(() => void result.current("/develop/3"));
    act(() => void window.dispatchEvent(new Event("pointerdown")));
    expect(calls()).toEqual([]);
  });

  it("reports a change of screen but not a change of photo inside it", () => {
    const { result } = setup(true, "/develop/1");
    expect(calls()).toEqual([{ screen: "developing" }]);
    act(() => void result.current("/develop/2"));
    expect(calls()).toHaveLength(1);
    act(() => void result.current("/library?asset=7"));
    expect(calls().at(-1)).toEqual({ screen: "viewing" });
  });

  it("reports a playing slideshow", () => {
    setup(true, "/slideshows/2");
    act(() => slideshowPlayingStore.set(true));
    expect(calls().at(-1)).toEqual({ screen: "slideshow" });
  });

  it("beats on input at most once a minute", () => {
    setup(true);
    expect(calls()).toHaveLength(1);
    act(() => void vi.advanceTimersByTime(10_000));
    act(() => void window.dispatchEvent(new Event("pointerdown")));
    expect(calls()).toHaveLength(1);
    act(() => void vi.advanceTimersByTime(51_000));
    act(() => void window.dispatchEvent(new Event("keydown")));
    expect(calls()).toHaveLength(2);
    expect(calls().at(-1)).toEqual({ screen: "browsing" });
  });

  it("sends the screen when switched on", () => {
    const { rerender } = setup(false);
    expect(calls()).toEqual([]);
    rerender({ on: true });
    expect(calls()).toEqual([{ screen: "browsing" }]);
  });
});
