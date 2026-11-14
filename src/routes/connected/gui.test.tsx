import { waitFor, act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { upsertJob } from "@/lib/jobs-store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { mockDefaults, mount } from "./test-utils";

const card = {
  kind: "volume",
  id: "E:\\",
  name: "EOS_DIGITAL",
  storage: { totalBytes: 64_000_000_000, freeBytes: 16_000_000_000 },
  totalItems: 312,
  newItems: 48,
  totalBytes: 40_000_000_000,
  newest: [
    { token: "1", fileName: "IMG_0312.CR3", modifiedAt: 1_700_000_300, video: false },
    { token: "2", fileName: "MVI_0311.MP4", modifiedAt: 1_700_000_200, video: true },
  ],
  removable: true,
};

beforeEach(mockDefaults);

describe("ConnectedRoute", () => {
  it("shows the card's newest photos and storage, and imports what is new", async () => {
    mockCommand("get_connected_device", () => card);
    mockCommand("start_volume_import", () => 21);
    mount(`/connected/volume/${encodeURIComponent("E:\\")}`);

    expect(await screen.findByRole("heading", { level: 1, name: "EOS_DIGITAL" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("get_connected_device", { kind: "volume", id: "E:\\" });
    // More on the card than the previews: the last square counts the rest instead.
    const thumbs = screen.getAllByRole("img");
    expect(thumbs.map((i) => i.getAttribute("alt"))).toEqual(["IMG_0312.CR3"]);
    expect(thumbs[0].getAttribute("src")).toContain("/device-thumb/1");
    expect(screen.getByLabelText("311 more on the device")).toHaveTextContent("+311more");
    // One bar: this card's photos and videos, everything else on it, and what is free.
    expect(screen.getByRole("progressbar", { name: "Photos and videos: 40 GB" })).toHaveAttribute("aria-valuenow", "63");
    expect(screen.getByRole("progressbar", { name: "Other: 8.0 GB" })).toHaveAttribute("aria-valuenow", "13");
    expect(screen.getByText("16 GB free of 64 GB")).toBeInTheDocument();
    expect(screen.getByText(/48 new of 312 items/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Import 48 new" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_volume_import", { mount: "E:\\", limitBytes: null }));
    act(() => upsertJob({ id: "import:21", kind: "import", label: "Importing 48 items", progress: 0.5, state: "running", detail: "IMG_0290.CR3" }));
    expect(await screen.findByRole("progressbar", { name: "Import progress" })).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByText("IMG_0290.CR3")).toBeInTheDocument();
  });

  it("opens the device browser from the count of the rest", async () => {
    mockCommand("get_connected_device", () => card);
    mockCommand("open_device_browser", () => ({ id: 3, name: "EOS_DIGITAL", total: 312, new: 48, totalBytes: 1, dateMin: null, dateMax: null, knowsImported: true, dating: false }));
    mockCommand("browse_device", () => ({ items: [], nextCursor: null, matching: 0, generation: 0 }));
    mockCommand("device_browser_ids", () => []);
    mockCommand("close_device_browser", () => null);
    mount(`/connected/volume/${encodeURIComponent("E:\\")}`);
    await userEvent.click(await screen.findByRole("button", { name: "311 more on the device" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("open_device_browser", { target: { kind: "volume", id: "E:\\" } }));
    expect(await screen.findByRole("dialog", { name: "EOS_DIGITAL" })).toBeInTheDocument();
  });

  it("puts Browse to the left of Import in one row, with Import everything again below", async () => {
    mockCommand("get_connected_device", () => card);
    mount(`/connected/volume/${encodeURIComponent("E:\\")}`);
    await screen.findByRole("button", { name: "Import 48 new" });
    const row = screen.getByRole("button", { name: "Browse…" }).closest(".connected-route__buttons");
    expect(row).not.toBeNull();
    const inRow = Array.from(row!.querySelectorAll("button")).map((b) => b.textContent);
    expect(inRow).toEqual(["Browse…", "Import 48 new"]);
    const again = screen.getByRole("button", { name: "Import everything again" });
    expect(row!.contains(again)).toBe(false);
    expect(row!.compareDocumentPosition(again) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("opens the device browser on arrival with ?browse=1, from the sidebar's menu (Round 4G)", async () => {
    mockCommand("get_connected_device", () => card);
    mockCommand("open_device_browser", () => ({ id: 3, name: "EOS_DIGITAL", total: 312, new: 48, totalBytes: 1, dateMin: null, dateMax: null, knowsImported: true, dating: false }));
    mockCommand("browse_device", () => ({ items: [], nextCursor: null, matching: 0, generation: 0 }));
    mockCommand("device_browser_ids", () => []);
    mockCommand("close_device_browser", () => null);
    mount(`/connected/volume/${encodeURIComponent("E:\\")}?browse=1`);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("open_device_browser", { target: { kind: "volume", id: "E:\\" } }));
  });

  it("says when the device has gone rather than showing an error", async () => {
    mockCommand("get_connected_device", () => {
      throw { code: "device_gone", message: "That card or drive is no longer connected." };
    });
    mount("/connected/volume/F%3A%5C");
    expect(await screen.findByText("That card or drive is no longer connected.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Go to Import" })).toHaveAttribute("href", "/import");
  });

  it("asks for Trust on a phone that has not granted it, with a way to try again", async () => {
    let calls = 0;
    mockCommand("get_connected_device", () => {
      calls++;
      if (calls === 1) throw { code: "iphone_trust_required", message: "unlock the phone and tap Trust" };
      return { ...card, kind: "iphone", id: "0000", name: "Phone", removable: true, storage: null, newItems: 0, totalItems: 2 };
    });
    mount("/connected/iphone/0000");
    expect(await screen.findByText(/tap Trust/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Phone" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Up to date" })).toBeDisabled();
    expect(screen.getAllByRole("img")).toHaveLength(2);
    expect(screen.queryByLabelText(/more on the device/)).not.toBeInTheDocument();
  });
});
