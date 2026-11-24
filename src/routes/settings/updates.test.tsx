import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { appCommands } from "@/lib/commands";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { UpdateState } from "@/types/generated/UpdateState";
import { SettingsRoute } from "./gui";
import { resetUpdateNotices } from "@/hooks/use-updates";
import { updateLine, useUpdateNotices } from "./updates";

const settings = (channel = "stable", automatic = true, security = true) => ({
  version: 1,
  appearance: { theme: "gallery", scheme: "auto", tileSize: "m", reduceMotion: false, sidebarCollapsed: false },
  import: { duplicates: "skip", promptOnVolume: true },
  playback: { loopVideos: false, muteByDefault: false },
  map: { online: true, style: "map" as const },
  music: { jamendoClientId: null },
  smart: { enabled: true, gpu: true, experimental: { autoModel: false, retouch: false } },
  window: { width: 1280, height: 800, x: null, y: null, maximized: false },
  updates: { automatic, channel, security },
});

function mount(state: UpdateState, s = settings()) {
  mockCommand("get_settings", () => s);
  mockCommand("update_settings", (a) => a?.settings);
  mockLibraryDefaults();
  mockCommand("update_status", () => ({ version: "26.1.0-alpha", state, whatsNew: null, security: false }));
  render(
    <Providers>
      <MemoryRouter initialEntries={["/settings?section=updates"]}>
        <SettingsRoute />
      </MemoryRouter>
    </Providers>,
  );
}

describe("updateLine", () => {
  it("says each state in one plain line", () => {
    expect(updateLine({ state: "off" })).toBe("Updates are off in development builds.");
    expect(updateLine({ state: "notSetUp" })).toBe("Updates are not set up in this build.");
    expect(updateLine({ state: "checking" })).toBe("Checking…");
    expect(updateLine({ state: "upToDate", checkedAt: 1 })).toMatch(/^Up to date/);
    expect(updateLine({ state: "downloading", version: "26.1.1-alpha", done: 42, total: 100 })).toBe("Downloading 26.1.1-alpha · 42 %");
    expect(updateLine({ state: "ready", version: "26.1.1-alpha", notes: "", attempts: 0 })).toBe("Ready: 26.1.1-alpha installs when you close Lumen.");
    expect(updateLine({ state: "ready", version: "26.1.1-alpha", notes: "", attempts: 2 })).toMatch(/last try to install it didn't finish/);
    expect(updateLine({ state: "available", version: "26.1.1-alpha", notes: "" })).toBe("26.1.1-alpha is available.");
    expect(updateLine({ state: "failed", why: "offline", at: 1 })).toMatch(/offline/i);
    // A working network with a dead feed is not "offline".
    expect(updateLine({ state: "failed", why: "noServer", at: 1 })).toBe("Could not check: Lumen can't reach the update server. It tries again later.");
    expect(updateLine({ state: "failed", why: "noServer", at: 1 })).not.toMatch(/offline/i);
    expect(updateLine({ state: "failed", why: "badSignature", at: 1 })).toMatch(/could not be verified/);
    expect(updateLine({ state: "failed", why: "noSpace", at: 1 })).toMatch(/space/);
  });
});

describe("Settings → Updates", () => {
  it("shows the version, automatic updates on and Stable by default, and says what the check sends", async () => {
    mount({ state: "upToDate", checkedAt: 1 });
    expect(await screen.findByText("Lumen 26.1.0-alpha")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Automatic updates" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Stable" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText(/sends no identifier and no usage data/)).toBeInTheDocument();
    expect(screen.getByText(/Stable gets its first build later/)).toBeInTheDocument();
  });

  it("warns inline, without a dialog, when Alpha or Beta is chosen", async () => {
    mount({ state: "upToDate", checkedAt: 1 });
    expect(screen.queryByText(/development builds\. They are unstable/)).toBeNull();
    await userEvent.click(await screen.findByRole("radio", { name: "Alpha" }));
    expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ updates: expect.objectContaining({ channel: "alpha" }) }) }));
    expect(await screen.findByText(/development builds\. They are unstable/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("checks when asked", async () => {
    mount({ state: "idle" });
    mockCommand("check_for_updates", () => ({ version: "26.1.0-alpha", state: { state: "upToDate", checkedAt: 2 }, whatsNew: null, security: false }));
    await userEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
    expect(invokeMock).toHaveBeenCalledWith("check_for_updates", undefined);
  });

  it("offers Install now once a build is ready", async () => {
    mount({ state: "ready", version: "26.1.1-alpha", notes: "Faster grid.", attempts: 0 });
    mockCommand("install_update_now", () => null);
    expect(await screen.findAllByText("Ready: 26.1.1-alpha installs when you close Lumen.")).toHaveLength(2); // the row and the inspector
    expect(screen.getByText("Faster grid.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Install now" }));
    expect(invokeMock).toHaveBeenCalledWith("install_update_now", undefined);
  });

  it("offers Download when automatic updates are off", async () => {
    mount({ state: "available", version: "26.1.1-alpha", notes: "" }, settings("alpha", false));
    mockCommand("download_update", () => ({ version: "26.1.0-alpha", state: { state: "downloading", version: "26.1.1-alpha", done: 0, total: 10 }, whatsNew: null, security: false }));
    await userEvent.click(await screen.findByRole("button", { name: "Download" }));
    expect(invokeMock).toHaveBeenCalledWith("download_update", undefined);
  });

  it("has Install security fixes automatically on by default, independent of Automatic updates", async () => {
    mount({ state: "upToDate", checkedAt: 1 }, settings("stable", false, true));
    const security = await screen.findByRole("switch", { name: "Install security fixes automatically" });
    expect(security).toBeChecked();
    expect(screen.getByRole("switch", { name: "Automatic updates" })).not.toBeChecked();
    await userEvent.click(security);
    expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ updates: expect.objectContaining({ security: false, automatic: false }) }) }));
  });

  it("keeps Automatic updates as it was when the security switch changes, and the reverse", async () => {
    mount({ state: "upToDate", checkedAt: 1 }, settings("stable", true, false));
    await userEvent.click(await screen.findByRole("switch", { name: "Automatic updates" }));
    expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.objectContaining({ settings: expect.objectContaining({ updates: expect.objectContaining({ automatic: false, security: false }) }) }));
  });

  it("checks nothing in a development build", async () => {
    mount({ state: "off" });
    expect(await screen.findAllByText("Updates are off in development builds.")).toHaveLength(2); // the row and the inspector
    expect(screen.queryByRole("button", { name: "Check for updates" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Check now" })).toBeNull();
  });
});

describe("palette", () => {
  it("has Check for updates", () => {
    expect(appCommands.some((c) => c.id === "check-updates" && c.label === "Check for updates")).toBe(true);
  });
});

describe("useUpdateNotices", () => {
  const wrap = ({ children }: { children: ReactNode }) => (
    <Providers>
      <MemoryRouter>{children}</MemoryRouter>
    </Providers>
  );

  it("says once that Lumen was updated", async () => {
    toastStore.set({ toasts: [] });
    let asked = 0;
    mockCommand("take_update_receipt", () => (asked++ === 0 ? { version: "26.1.4-alpha", notes: "Notes" } : null));
    const { rerender } = renderHook(() => useUpdateNotices(), { wrapper: wrap });
    await waitFor(() => expect(toastStore.get().toasts.map((t) => t.message)).toContain("Updated to 26.1.4-alpha"));
    rerender();
    await act(async () => {});
    expect(toastStore.get().toasts.filter((t) => t.message === "Updated to 26.1.4-alpha")).toHaveLength(1);
  });

  it("says Lumen installed a security fix after one", async () => {
    resetUpdateNotices();
    toastStore.set({ toasts: [] });
    mockCommand("take_update_receipt", () => ({ version: "26.3.5-stable", notes: "", security: true }));
    renderHook(() => useUpdateNotices(), { wrapper: wrap });
    await waitFor(() => expect(toastStore.get().toasts.map((t) => t.message)).toContain("Lumen installed a security fix (26.3.5-stable)"));
    expect(toastStore.get().toasts.map((t) => t.message)).not.toContain("Updated to 26.3.5-stable");
  });
});
