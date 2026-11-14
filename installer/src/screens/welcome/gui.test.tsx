import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import { Welcome } from "./index";

describe("Welcome", () => {
  const base = { version: "26.1.2-alpha", sizeOnDisk: 288 * 1024 ** 2, onInstall: vi.fn(), onOptions: vi.fn() };

  it("offers Install and Options and states version, size and privacy", async () => {
    const onInstall = vi.fn();
    const onOptions = vi.fn();
    render(<Welcome {...base} onInstall={onInstall} onOptions={onOptions} />);
    expect(screen.getByText("Your photos, on your machine, fast.")).toBeInTheDocument();
    expect(screen.getByText(/26\.1\.2-alpha/)).toBeInTheDocument();
    expect(screen.getByText(/288 MB on this PC/)).toBeInTheDocument();
    expect(screen.getByText(/no account, nothing leaves your computer/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    await userEvent.click(screen.getByRole("button", { name: "Options…" }));
    expect(onInstall).toHaveBeenCalledOnce();
    expect(onOptions).toHaveBeenCalledOnce();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = render(<Welcome {...base} />);
    await expectEveryControlReachableByTab(container);
  });
});
