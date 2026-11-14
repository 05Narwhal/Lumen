import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import type { FolderCheck } from "@/types";
import { Options } from "./index";

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const ok: FolderCheck = { kind: "ok", free: 214 * GB };

function setup(over: Partial<ComponentProps<typeof Options>> = {}) {
  const props: ComponentProps<typeof Options> = {
    folder: "C:\\Users\\Sample\\AppData\\Local\\Lumen",
    check: ok,
    desktopShortcut: true,
    startWithWindows: false,
    downloadModels: true,
    modelsBytes: 500 * MB,
    needBytes: 288 * MB,
    onChange: vi.fn(),
    onChangeFolder: vi.fn(),
    onBack: vi.fn(),
    onInstall: vi.fn(),
    ...over,
  };
  return { props, ...render(<Options {...props} />) };
}

describe("Options", () => {
  it("starts with the defaults on, off, on and the real model size", () => {
    setup();
    expect(screen.getByRole("switch", { name: "Desktop shortcut" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Start with Windows" })).not.toBeChecked();
    expect(screen.getByRole("switch", { name: "Download smart features now" })).toBeChecked();
    expect(screen.getByText(/500 MB: search by description/)).toBeInTheDocument();
    expect(screen.getByText(/Needs 788 MB · 214 GB free/)).toBeInTheDocument();
  });

  it("reports a switch change", async () => {
    const { props } = setup();
    await userEvent.click(screen.getByRole("switch", { name: "Start with Windows" }));
    expect(props.onChange).toHaveBeenCalledWith({ startWithWindows: true });
  });

  it("says what Off means for the models and drops them from the space needed", () => {
    setup({ downloadModels: false });
    expect(screen.getByText("Off: Lumen downloads them the first time it opens.")).toBeInTheDocument();
    expect(screen.getByText(/Needs 288 MB · 214 GB free/)).toBeInTheDocument();
  });

  it.each([
    ["needs_admin", /needs administrator rights/],
    ["not_empty", /other files in it/],
    ["no_space", /Not enough space/],
    ["is_file", /is a file/],
  ] as const)("install_is_disabled_for_each_folder_problem: %s", async (kind, reason) => {
    const { props } = setup({ check: { kind, free: kind === "no_space" ? 100 * MB : 214 * GB } });
    expect(screen.getByRole("alert")).toHaveTextContent(reason);
    const install = screen.getByRole("button", { name: "Install" });
    expect(install).toBeDisabled();
    await userEvent.click(install);
    expect(props.onInstall).not.toHaveBeenCalled();
  });

  it("enables Install for a good folder and goes back", async () => {
    const { props } = setup();
    expect(screen.queryByRole("alert")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    await userEvent.click(screen.getByRole("button", { name: "Change…" }));
    expect(props.onInstall).toHaveBeenCalledOnce();
    expect(props.onBack).toHaveBeenCalledOnce();
    expect(props.onChangeFolder).toHaveBeenCalledOnce();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = setup();
    await expectEveryControlReachableByTab(container);
  });
});
