import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { setKeyOverrides } from "./keybinds";
import { applyMark, markForKey } from "./marks";

const key = (over: Partial<KeyboardEvent>) => ({ key: "", code: "", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, ...over });

describe("markForKey", () => {
  it("reads the culling keys", () => {
    expect(markForKey(key({ key: "p" }))).toEqual({ kind: "flag", value: 1 });
    expect(markForKey(key({ key: "X" }))).toEqual({ kind: "flag", value: -1 });
    expect(markForKey(key({ key: "u" }))).toEqual({ kind: "flag", value: 0 });
    expect(markForKey(key({ key: "¡", code: "Digit1", altKey: true }))).toEqual({ kind: "label", value: 1 });
    expect(markForKey(key({ code: "Digit0", altKey: true }))).toEqual({ kind: "label", value: 0 });
  });

  it("follows the person's own keys", () => {
    setKeyOverrides({ "develop.pick": ["Q"] });
    try {
      expect(markForKey(key({ key: "q" }))).toEqual({ kind: "flag", value: 1 });
      expect(markForKey(key({ key: "p" }))).toBeNull();
    } finally {
      setKeyOverrides(undefined);
    }
  });

  it("leaves everything else alone", () => {
    expect(markForKey(key({ key: "p", ctrlKey: true }))).toBeNull();
    expect(markForKey(key({ key: "1", code: "Digit1" }))).toBeNull();
    expect(markForKey(key({ code: "Digit7", altKey: true }))).toBeNull();
    expect(markForKey(key({ key: "f" }))).toBeNull();
  });
});

describe("applyMark", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    toastStore.set({ toasts: [] });
  });

  it("undoes to what each photo had before", async () => {
    mockCommand("set_color_label", () => [
      { id: 1, previous: 0 },
      { id: 2, previous: 4 },
      { id: 3, previous: 0 },
    ]);
    await applyMark(new QueryClient(), "label", [1, 2, 3], 3);
    expect(invokeMock).toHaveBeenCalledWith("set_color_label", { ids: [1, 2, 3], label: 3 });
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("Labelled 3 items green");
    invokeMock.mockClear();
    t.action!.run();
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2));
    expect(invokeMock).toHaveBeenCalledWith("set_color_label", { ids: [1, 3], label: 0 });
    expect(invokeMock).toHaveBeenCalledWith("set_color_label", { ids: [2], label: 4 });
  });

  it("says nothing when nothing changed", async () => {
    mockCommand("set_flag", () => []);
    await applyMark(new QueryClient(), "flag", [5], 1);
    expect(invokeMock).toHaveBeenCalledWith("set_flag", { ids: [5], flag: 1 });
    expect(toastStore.get().toasts).toHaveLength(0);
  });
});
