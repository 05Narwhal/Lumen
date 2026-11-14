import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import { Done } from "./index";

describe("Done", () => {
  it("says everything is installed when the models arrived", () => {
    render(<Done models="complete" openNow onToggleOpen={vi.fn()} onFinish={vi.fn()} />);
    expect(screen.getByText("Lumen is ready")).toBeInTheDocument();
    expect(screen.getByText(/Everything is installed\./)).toBeInTheDocument();
  });

  it("says the models finish later when they did not", () => {
    render(<Done models="later" openNow onToggleOpen={vi.fn()} onFinish={vi.fn()} />);
    expect(screen.getByText(/Smart features will finish downloading the next time Lumen is open\./)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("ticks Open Lumen now and finishes", async () => {
    const onToggleOpen = vi.fn();
    const onFinish = vi.fn();
    render(<Done models="complete" openNow onToggleOpen={onToggleOpen} onFinish={onFinish} />);
    const box = screen.getByRole("checkbox", { name: "Open Lumen now" });
    expect(box).toBeChecked();
    await userEvent.click(box);
    await userEvent.click(screen.getByRole("button", { name: "Finish" }));
    expect(onToggleOpen).toHaveBeenCalledWith(false);
    expect(onFinish).toHaveBeenCalledOnce();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = render(<Done models="complete" openNow onToggleOpen={vi.fn()} onFinish={vi.fn()} />);
    await expectEveryControlReachableByTab(container);
  });
});
