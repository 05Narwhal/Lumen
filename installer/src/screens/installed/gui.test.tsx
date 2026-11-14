import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import { Installed } from "./index";

const handlers = () => ({ onPrimary: vi.fn(), onOpen: vi.fn(), onRemove: vi.fn() });
const at = (kind: "older" | "same" | "newer", version: string) => ({ kind, version, location: "C:\\Lumen" }) as const;

describe("Installed", () => {
  it("offers Update to X over an older install and keeps nothing else on screen", async () => {
    const h = handlers();
    render(<Installed installed={at("older", "26.1.0-alpha")} ours="26.1.2-alpha" lumenRunning={false} {...h} />);
    expect(screen.getByText("Update to 26.1.2-alpha")).toBeInTheDocument();
    expect(screen.getByText(/Keeps your settings, libraries and shortcuts\./)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Update" }));
    expect(h.onPrimary).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "Open Lumen" })).toBeNull();
  });

  it("offers Repair over the same version", async () => {
    const h = handlers();
    render(<Installed installed={at("same", "26.1.2-alpha")} ours="26.1.2-alpha" lumenRunning={false} {...h} />);
    await userEvent.click(screen.getByRole("button", { name: "Repair" }));
    expect(h.onPrimary).toHaveBeenCalledOnce();
  });

  it("never downgrades: a newer install offers Open Lumen only", async () => {
    const h = handlers();
    render(<Installed installed={at("newer", "26.2.0")} ours="26.1.2-alpha" lumenRunning={false} {...h} />);
    expect(screen.getByText("This PC already has a newer Lumen")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Update" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Repair" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Open Lumen" }));
    expect(h.onOpen).toHaveBeenCalledOnce();
  });

  it.each(["older", "same", "newer"] as const)("offers Remove… over a %s install", async (kind) => {
    const h = handlers();
    render(<Installed installed={at(kind, "26.1.0")} ours="26.1.0" lumenRunning={false} {...h} />);
    await userEvent.click(screen.getByRole("button", { name: "Remove…" }));
    expect(h.onRemove).toHaveBeenCalledOnce();
  });

  it("says Lumen closes itself and reopens when it is running", () => {
    render(<Installed installed={at("older", "26.1.0")} ours="26.1.2" lumenRunning {...handlers()} />);
    expect(screen.getByText("Lumen is open. It closes itself before the update and reopens after.")).toBeInTheDocument();
  });

  it("does not talk about closing when a newer Lumen is running (Setup changes nothing)", () => {
    render(<Installed installed={at("newer", "26.2.0")} ours="26.1.2" lumenRunning {...handlers()} />);
    expect(screen.queryByText(/closes itself/)).toBeNull();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = render(<Installed installed={at("older", "26.1.0")} ours="26.1.2" lumenRunning {...handlers()} />);
    await expectEveryControlReachableByTab(container);
  });
});
