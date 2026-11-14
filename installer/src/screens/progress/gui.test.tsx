import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import { Progress } from "./index";

const MB = 1024 ** 2;
const base = {
  heading: "Installing Lumen",
  lumen: { state: "running" as const, done: 3, total: 10 },
  models: { state: "waiting" as const, done: 0, total: 500 * MB },
  error: null,
  onOpen: vi.fn(),
  onSkip: vi.fn(),
  onRetry: vi.fn(),
};

describe("Progress", () => {
  it("shows two tasks and keeps Open Lumen off until Lumen is installed", () => {
    render(<Progress {...base} />);
    expect(screen.getByText("Lumen")).toBeInTheDocument();
    expect(screen.getByText("Smart features")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Lumen" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Skip smart features" })).toBeNull();
  });

  it("opens Lumen and skips the download once Lumen is done", async () => {
    const onOpen = vi.fn();
    const onSkip = vi.fn();
    render(
      <Progress
        {...base}
        onOpen={onOpen}
        onSkip={onSkip}
        lumen={{ state: "done", done: 10, total: 10 }}
        models={{ state: "running", done: 212 * MB, total: 500 * MB, secondsLeft: 70 }}
      />,
    );
    expect(screen.getByText(/212 of 500 MB · 1 min left/)).toBeInTheDocument();
    expect(screen.getByText(/You can open Lumen now/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Open Lumen" }));
    await userEvent.click(screen.getByRole("button", { name: "Skip smart features" }));
    expect(onOpen).toHaveBeenCalledOnce();
    expect(onSkip).toHaveBeenCalledOnce();
  });

  it("leaves the smart features row out when they are off", () => {
    render(<Progress {...base} models={null} />);
    expect(screen.queryByText("Smart features")).toBeNull();
  });

  it("names what happened and offers Try again when the install fails", async () => {
    const onRetry = vi.fn();
    render(
      <Progress
        {...base}
        onRetry={onRetry}
        lumen={{ state: "failed", done: 3, total: 10 }}
        error="The installer stopped with code 2. Nothing was changed; try again."
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("The installer stopped with code 2.");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("shows the Lumen row as waiting, not installing, until Lumen has closed", () => {
    const running = { ...base, lumen: { state: "running" as const, done: 0, total: 100 } };
    const { rerender } = render(<Progress {...running} closing="asking-import" />);
    expect(screen.getByText("waiting for Lumen to close")).toBeInTheDocument();
    expect(screen.queryByText("installing")).not.toBeInTheDocument();
    expect(screen.queryByRole("progressbar", { name: "Installing Lumen" })).not.toBeInTheDocument();
    rerender(<Progress {...running} closing="closed" />);
    expect(screen.getByText("installing")).toBeInTheDocument();
  });

  it("says Lumen is closing, then that it asks about the import, with Cancel", async () => {
    const onCancel = vi.fn();
    const { rerender } = render(<Progress {...base} closing="closing" onCancel={onCancel} />);
    expect(screen.getByRole("status")).toHaveTextContent("Closing Lumen…");
    rerender(<Progress {...base} closing="asking-import" onCancel={onCancel} />);
    expect(screen.getByRole("status")).toHaveTextContent(
      "Lumen is asking about the import that's running. Answer there and Setup continues.",
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
    rerender(<Progress {...base} closing="closed" onCancel={onCancel} />);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = render(
      <Progress
        {...base}
        lumen={{ state: "done", done: 10, total: 10 }}
        models={{ state: "running", done: 1, total: 2 }}
      />,
    );
    await expectEveryControlReachableByTab(container);
  });
});
