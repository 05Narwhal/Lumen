import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { mockDefaults, mount } from "./test-utils";

const LIBRARY = "/Photo Library/Julian.photoslibrary";

beforeEach(mockDefaults);

describe("Mac disk page", () => {
  it("shows a Mac disk's libraries and lets you import a folder from it", async () => {
    mockCommand("get_connected_device", () => ({ kind: "mac", id: "disk3-part2", name: "ADATA HD710 PRO", storage: { totalBytes: 2_000_000_000_000, freeBytes: 250_000_000_000 }, totalItems: 0, newItems: 0, totalBytes: 0, newest: [], removable: true }));
    mockCommand("list_mac_disks", () => [{ id: "disk3-part2", name: "ADATA HD710 PRO", fs: "hfs", totalBytes: 2_000_000_000_000, freeBytes: 250_000_000_000, libraries: [LIBRARY] }]);
    mockCommand("get_photos_library", () => ({
      summary: { name: "Julian", items: 34481, livePhotos: 728, notDownloaded: 0, referenced: 0, burstFrames: 4280, trashed: 0 },
      totalFiles: 35209,
      newItems: 35209,
      newest: [{ token: "5", fileName: "IMG_1.HEIC", modifiedAt: null, video: false }],
    }));
    mockCommand("list_mac_folder", (a) =>
      a?.path === "/" ? { path: "/", folders: ["mami photos", "Photo Library"], mediaHere: 0 } : { path: "/mami photos", folders: [], mediaHere: 212 },
    );
    mockCommand("start_mac_folder_import", () => 31);
    mockCommand("start_photos_library_import", () => 32);
    mount("/connected/mac/disk3-part2");

    expect(await screen.findByRole("heading", { level: 1, name: "ADATA HD710 PRO" })).toBeInTheDocument();
    expect(await screen.findByText("Julian")).toBeInTheDocument();
    expect(await screen.findByText(/34,481 photos and videos · 728 Live Photos/)).toBeInTheDocument();
    expect(screen.getByText(/4,280 unpicked burst frames/)).toBeInTheDocument();
    expect(screen.getByText("Mac disk · HFS+")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Import library" }));
    expect(invokeMock).toHaveBeenCalledWith("start_photos_library_import", { disk: "disk3-part2", path: LIBRARY });

    await userEvent.click(await screen.findByRole("button", { name: /mami photos/ }));
    expect(await screen.findByText(/212 photos and videos here/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "mami photos" })).toHaveAttribute("aria-current", "location");
    await userEvent.click(screen.getByRole("switch", { name: "Include subfolders" }));
    await userEvent.click(screen.getByRole("button", { name: "Import this folder" }));
    expect(invokeMock).toHaveBeenCalledWith("start_mac_folder_import", { disk: "disk3-part2", path: "/mami photos", recursive: false });
  });

  it("opens a library picked on this computer and says what stays behind", async () => {
    mockCommand("get_photos_library", () => ({
      summary: { name: "Photos Library", items: 12, livePhotos: 0, notDownloaded: 3, referenced: 2, burstFrames: 0, trashed: 0 },
      totalFiles: 12,
      newItems: 5,
      newest: [],
    }));
    const path = "D:/Backup/Photos Library.photoslibrary";
    mount(`/connected/photos/${encodeURIComponent(path)}`);
    expect(await screen.findByRole("heading", { level: 1, name: "Photos Library" })).toBeInTheDocument();
    expect(await screen.findByText(/3 not downloaded from iCloud · 2 referenced/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import 5 new" })).toBeEnabled();
    // Browse… first, then the primary Import, in one row (Round 4E Task 6).
    const row = screen.getByRole("button", { name: "Browse…" }).closest(".connected-route__buttons");
    expect(Array.from(row!.querySelectorAll("button")).map((b) => b.textContent)).toEqual(["Browse…", "Import 5 new"]);
    expect(invokeMock).toHaveBeenCalledWith("get_photos_library", { disk: null, path });
  });
});
