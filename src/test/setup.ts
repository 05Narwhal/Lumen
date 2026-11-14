import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import { resetInvokeMock } from "./tauri-mock";

// findBy/waitFor give up after 1 s by default, which a Mantine render can take when the whole
// suite shares the machine. Five seconds only slows a test that is failing anyway.
configure({ asyncUtilTimeout: 5000 });

// jsdom lacks matchMedia; Mantine's colour-scheme manager needs it.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

window.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

beforeEach(() => resetInvokeMock());
afterEach(() => cleanup());

// jsdom implements neither of these and throws, which the virtual console then reports for
// every call. The music picker plays previews, so without this the whole suite pays for a
// stack trace per click — slow enough to push other files' waitFor budgets over.
Object.defineProperty(HTMLMediaElement.prototype, "play", {
  configurable: true,
  value: () => Promise.resolve(),
});
Object.defineProperty(HTMLMediaElement.prototype, "pause", {
  configurable: true,
  value: () => {},
});

// jsdom lacks scrollIntoView; Mantine's Select calls it when an option is highlighted.
Element.prototype.scrollIntoView = () => {};
