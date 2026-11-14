import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import type { BrowserFilter } from "@/types/generated/BrowserFilter";
import type { BrowserItem } from "@/types/generated/BrowserItem";
import { DeviceBrowser } from "./gui";

const DAY = 86_400;
// Wall times: 30 items a day for ten days, newest first, ending 18 March 2026 at noon.
const END = Date.UTC(2026, 2, 18, 12) / 1000;
const ALL: BrowserItem[] = Array.from({ length: 300 }, (_, i) => ({
  index: i,
  token: String(1000 + i),
  fileName: `IMG_${String(i).padStart(4, "0")}.JPG`,
  size: 1_000_000,
  files: 1,
  date: END - Math.floor(i / 30) * DAY - (i % 30) * 60,
  dateSource: "exif",
  kind: "photo",
  imported: i >= 290,
}));

const matches = (f: BrowserFilter) => ALL.filter((it) => (f.from === null || it.date! >= f.from) && (f.to === null || it.date! <= f.to) && (!f.newOnly || !it.imported));

function fakeSession() {
  mockCommand("get_settings", () => null);
  mockCommand("open_device_browser", () => ({
    id: 7,
    name: "EOS_DIGITAL",
    total: 300,
    new: 290,
    totalBytes: 300_000_000,
    dateMin: ALL[299].date,
    dateMax: ALL[0].date,
    knowsImported: true,
    dating: false,
  }));
  mockCommand("browse_device", (a) => {
    const f = a!.filter as BrowserFilter;
    const all = matches(f);
    const start = (a!.cursor as number | null) ?? 0;
    const end = Math.min(all.length, start + (a!.limit as number));
    return { items: all.slice(start, end), nextCursor: end < all.length ? end : null, matching: all.length, generation: 0 };
  });
  mockCommand("device_browser_ids", (a) => matches(a!.filter as BrowserFilter).map((i) => i.index));
  mockCommand("device_browser_selection", (a) => {
    const picked = ALL.filter((i) => (a!.indexes as number[]).includes(i.index));
    return { count: picked.length, files: picked.length, bytes: picked.length * 1_000_000, imported: picked.filter((i) => i.imported).length };
  });
  mockCommand("get_device_browser", () => ({ id: 7, name: "EOS_DIGITAL", total: 300, new: 290, totalBytes: 300_000_000, dateMin: null, dateMax: null, knowsImported: true, dating: false }));
  mockCommand("close_device_browser", () => null);
  mockCommand("plan_device_browser_import", (a) => {
    const count = (a!.indexes as number[]).length;
    return { count, bytes: count, freeBytes: 1e12, fitsCount: count, fitsBytes: count, buffer: 20e9 };
  });
  mockCommand("start_device_browser_import", () => 33);
}

const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;

function mount(onClose = vi.fn(), onImportStarted = vi.fn()) {
  render(
    <Providers>
      <DeviceBrowser target={{ kind: "volume", id: "E:\\" }} title="EOS_DIGITAL" onClose={onClose} onImportStarted={onImportStarted} />
    </Providers>,
  );
  return { onClose, onImportStarted };
}

const bar = () => screen.getByText(/selected/, { selector: ".device-browser__count" });

describe("DeviceBrowser", () => {
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
    vi.useFakeTimers({ toFake: ["Date"], now: new Date(2026, 2, 18, 15) });
    fakeSession();
  });
  afterEach(() => {
    vi.useRealTimers();
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  it("opens a session and shows its first page by day", async () => {
    mount();
    expect(await screen.findByRole("button", { name: "IMG_0000.JPG" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("open_device_browser", { target: { kind: "volume", id: "E:\\" } });
    expect(invokeMock).toHaveBeenCalledWith("browse_device", expect.objectContaining({ session: 7, cursor: null, limit: 200 }));
    expect(screen.getByText(/300 of 300/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 3, name: /18 March 2026|March 18, 2026/ })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "IMG_0000.JPG" }).getAttribute("src")).toContain("/device-thumb/1000");
  });

  it("filters to this month with a quick pick", async () => {
    mount();
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    await userEvent.click(screen.getByRole("button", { name: "This month" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "browse_device",
        expect.objectContaining({ filter: expect.objectContaining({ from: Date.UTC(2026, 2, 1) / 1000, to: Date.UTC(2026, 2, 19) / 1000 - 1 }) }),
      ),
    );
  });

  it("selects all under a filter, keeps the selection across filters and imports exactly it", async () => {
    const { onImportStarted, onClose } = mount();
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    // Today only: the first 30.
    await userEvent.click(screen.getByRole("button", { name: "Today" }));
    await screen.findByRole("button", { name: "Select all (30)" });
    await userEvent.click(screen.getByRole("button", { name: "Select all (30)" }));
    expect(invokeMock).toHaveBeenCalledWith("device_browser_ids", expect.objectContaining({ session: 7 }));
    await waitFor(() => expect(bar()).toHaveTextContent("30 selected"));
    // Everything again: still 30 selected, and the rest are not.
    await userEvent.click(screen.getByRole("button", { name: "All" }));
    await screen.findByRole("button", { name: "Select all (300)" });
    expect(bar()).toHaveTextContent("30 selected");
    // Back to yesterday: the 30 are hidden but still counted.
    await userEvent.click(screen.getByRole("button", { name: "IMG_0030.JPG" }));
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2026-03-17" } });
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "2026-03-17" } });
    await waitFor(() => expect(bar()).toHaveTextContent("31 selected"));
    await waitFor(() => expect(bar()).toHaveTextContent("… and 30 not shown"));

    await userEvent.click(screen.getByRole("button", { name: "Import 31" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_device_browser_import", expect.objectContaining({ session: 7, limitBytes: null })));
    const call = invokeMock.mock.calls.find((c) => c[0] === "start_device_browser_import")!;
    expect([...(call[1]!.indexes as number[])].sort((a, b) => a - b)).toEqual([...Array.from({ length: 30 }, (_, i) => i), 30]);
    expect(invokeMock).toHaveBeenCalledWith("plan_device_browser_import", expect.objectContaining({ session: 7 }));
    expect(onImportStarted).toHaveBeenCalledWith(33);
    expect(onClose).toHaveBeenCalled();
  });

  it("selects a range with Shift and says what is already imported", async () => {
    mount();
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    await userEvent.click(screen.getByLabelText("New only"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Select all (290)" })).toBeInTheDocument());
    await userEvent.click(screen.getByLabelText("New only"));
    await screen.findByRole("button", { name: "Select all (300)" });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "IMG_0002.JPG" }));
    await user.keyboard("{Shift>}");
    await user.click(screen.getByRole("button", { name: "IMG_0005.JPG" }));
    await user.keyboard("{/Shift}");
    await waitFor(() => expect(bar()).toHaveTextContent("4 selected"));
    expect(screen.getByRole("button", { name: "IMG_0003.JPG" })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows the gone state when the card is pulled out", async () => {
    mount();
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    act(() => emitMock("device://volumes", [{ mount: "F:\\", label: "OTHER", key: "k", totalItems: 0, newItems: 0, removable: true }]));
    expect(await screen.findByText("The device was disconnected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import" })).toBeDisabled();
  });

  it("refetches when the backend re-sorts the session", async () => {
    mount();
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    const before = invokeMock.mock.calls.filter((c) => c[0] === "browse_device").length;
    act(() => emitMock("device-browser://changed", { session: 7, generation: 1, dating: false }));
    await waitFor(() => expect(invokeMock.mock.calls.filter((c) => c[0] === "browse_device").length).toBeGreaterThan(before));
  });

  it("closes on Escape and ends the session", async () => {
    const { onClose } = mount();
    const tile = await screen.findByRole("button", { name: "IMG_0000.JPG" });
    tile.focus();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("ends the session when it unmounts", async () => {
    const { unmount } = render(
      <Providers>
        <DeviceBrowser target={{ kind: "volume", id: "E:\\" }} title="EOS_DIGITAL" onClose={vi.fn()} />
      </Providers>,
    );
    await screen.findByRole("button", { name: "IMG_0000.JPG" });
    unmount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("close_device_browser", { session: 7 }));
  });
});
