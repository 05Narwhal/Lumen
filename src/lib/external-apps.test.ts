import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it } from "vitest";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { ExternalApp } from "@/types/generated/ExternalApp";
import { appsFor, defaultFor, editIn, mediaOf } from "./external-apps";
import { queryKeys } from "./query-keys";

const app = (id: string, name: string, accepts: ExternalApp["accepts"]): ExternalApp => ({ id, name, kind: "editor", accepts, iconUrl: `photos://app-icon/${id}`, custom: false, send: "automatic", writes: "inPlace", addedBy: null });
const photo = { mediaType: 0, raw: false };
const raw = { mediaType: 0, raw: true };
const live = { mediaType: 2, raw: false };
const video = { mediaType: 1, raw: false };

describe("what a selection is", () => {
  it("names photo, raw, video or mixed (a Live Photo is a photo)", () => {
    expect(mediaOf([photo, live])).toBe("photo");
    expect(mediaOf([raw, raw])).toBe("raw");
    expect(mediaOf([video])).toBe("video");
    expect(mediaOf([photo, raw])).toBe("mixed");
    expect(mediaOf([photo, video])).toBe("mixed");
  });

  it("lists only the apps that take every kind in it", () => {
    const apps = [app("ps", "Photoshop", ["photo", "raw"]), app("vlc", "VLC", ["video"]), app("gimp", "GIMP", ["photo"])];
    expect(appsFor(apps, [photo]).map((a) => a.id)).toEqual(["ps", "gimp"]);
    expect(appsFor(apps, [photo, raw]).map((a) => a.id)).toEqual(["ps"]);
    expect(appsFor(apps, [photo, video])).toEqual([]);
  });

  it("picks the default for the media, and for a mix the first default that takes all", () => {
    const apps = [app("ps", "Photoshop", ["photo", "raw"]), app("gimp", "GIMP", ["photo"])];
    const defaults = { photo: "gimp", raw: "ps", video: null };
    expect(defaultFor([photo], apps, defaults)?.id).toBe("gimp");
    expect(defaultFor([raw], apps, defaults)?.id).toBe("ps");
    expect(defaultFor([photo, raw], apps, defaults)?.id).toBe("ps");
    expect(defaultFor([video], apps, defaults)).toBeNull();
    expect(defaultFor([photo], apps, { photo: "gone", raw: null, video: null })).toBeNull();
  });
});

describe("Edit in", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    toastStore.set({ toasts: [] });
    mockCommand("get_asset", (a) => ({ id: a?.id, filename: "IMG_0042.JPG" }));
    mockCommand("list_external_apps", () => []);
  });

  it("says how many photos it opens, or the one's name", async () => {
    mockCommand("open_in_app", () => ({ opened: 2, made: 2 }));
    const qc = new QueryClient();
    await editIn(qc, [1, 2], app("ps", "Photoshop", ["photo"]));
    expect(invokeMock).toHaveBeenCalledWith("open_in_app", { assetIds: [1, 2], appId: "ps" });
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Opening 2 photos in Photoshop");
    await editIn(qc, [7], app("ps", "Photoshop", ["photo"]));
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Opening IMG_0042.JPG in Photoshop");
  });

  it("tells when the app has gone and asks for the list again", async () => {
    mockCommand("open_in_app", () => {
      throw { code: "app_missing", message: "Photoshop isn't installed any more" };
    });
    const qc = new QueryClient();
    qc.setQueryData(queryKeys.externalApps, [app("ps", "Photoshop", ["photo"])]);
    await editIn(qc, [1], app("ps", "Photoshop", ["photo"]));
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Photoshop isn't installed any more");
    expect(qc.getQueryState(queryKeys.externalApps)?.isInvalidated).toBe(true);
  });
});
