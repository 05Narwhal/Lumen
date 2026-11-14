import { fireEvent, render, screen } from "@testing-library/react";
import { MotionGlobalConfig } from "framer-motion";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { AiButton, type AiButtonState } from "./gui";

beforeAll(() => {
  MotionGlobalConfig.skipAnimations = true;
});

function setup(state: AiButtonState | null, disabled = false) {
  const onRun = vi.fn();
  const onCancel = vi.fn();
  render(<AiButton label="Auto" state={state} onRun={onRun} onCancel={onCancel} disabled={disabled} />);
  return { onRun, onCancel, button: screen.getByRole("button") };
}

describe("AiButton", () => {
  it("idle shows its label and runs on click", () => {
    const { onRun, onCancel, button } = setup(null);
    expect(button).toHaveTextContent("Auto");
    expect(button).not.toHaveAttribute("data-busy");
    fireEvent.click(button);
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("looks idle while the task has not passed its delay", () => {
    const { button } = setup({ visible: false, stage: "Working…", done: 0, total: 0 });
    expect(button).not.toHaveAttribute("data-busy");
    expect(button).not.toHaveAttribute("aria-busy", "true");
    expect(button).toHaveTextContent("Auto");
    expect(button).not.toHaveTextContent("Working");
  });

  it("shows the stage, the ring value and aria-busy once visible", () => {
    const { button } = setup({ visible: true, stage: "Trying looks 2/3", done: 2, total: 3 });
    expect(button).toHaveTextContent("Trying looks 2/3");
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button).toHaveAttribute("data-busy");
    expect(button).not.toHaveAttribute("data-indeterminate");
    expect(button.style.getPropertyValue("--ai-progress")).toBe("0.667");
  });

  it("is a shimmer, not a ring, while the total is unknown", () => {
    const { button } = setup({ visible: true, stage: "Working…", done: 0, total: 0 });
    expect(button).toHaveAttribute("data-indeterminate");
    expect(button).toHaveTextContent("Working…");
  });

  it("keeps updating the stage text in place", () => {
    const { rerender } = render(
      <AiButton label="Auto" state={{ visible: true, stage: "Reading photo…", done: 0, total: 0 }} onRun={vi.fn()} onCancel={vi.fn()} />,
    );
    expect(screen.getByRole("button")).toHaveTextContent("Reading photo…");
    rerender(<AiButton label="Auto" state={{ visible: true, stage: "Applying…", done: 0, total: 0 }} onRun={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole("button")).toHaveTextContent("Applying…");
  });

  it("a click while busy cancels and never runs again", () => {
    const { onRun, onCancel, button } = setup({ visible: true, stage: "Working…", done: 0, total: 0 });
    fireEvent.click(button);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it("a second click before the delay is a cancel too", () => {
    const { onRun, onCancel, button } = setup({ visible: false, stage: "Working…", done: 0, total: 0 });
    fireEvent.click(button);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it("offers Cancel on hover and on focus while busy", () => {
    const { button } = setup({ visible: true, stage: "Trying looks 2/3", done: 2, total: 3 });
    fireEvent.mouseEnter(button);
    expect(button).toHaveTextContent("Cancel");
    fireEvent.mouseLeave(button);
    expect(button).toHaveTextContent("Trying looks 2/3");
    fireEvent.focus(button);
    expect(button).toHaveTextContent("Cancel");
  });

  it("shows the stage, not Cancel, when the pointer was already over the button as it started", () => {
    const idle = <AiButton label="Denoise" state={null} onRun={vi.fn()} onCancel={vi.fn()} />;
    const { rerender } = render(idle);
    const button = screen.getByRole("button");
    fireEvent.mouseEnter(button);
    rerender(<AiButton label="Denoise" state={{ visible: true, stage: "Denoising", done: 25, total: 100 }} onRun={vi.fn()} onCancel={vi.fn()} />);
    expect(button).toHaveTextContent("Denoising");
    fireEvent.mouseLeave(button);
    fireEvent.mouseEnter(button);
    expect(button).toHaveTextContent("Cancel");
  });

  it("without a cancel it neither offers Cancel nor reacts to a click while busy", () => {
    const onRun = vi.fn();
    const onCancel = vi.fn();
    render(
      <AiButton label="Auto" state={{ visible: true, stage: "Working…", done: 0, total: 0 }} cancellable={false} onRun={onRun} onCancel={onCancel} />,
    );
    const button = screen.getByRole("button");
    fireEvent.mouseEnter(button);
    expect(button).toHaveTextContent("Working…");
    fireEvent.click(button);
    expect(onCancel).not.toHaveBeenCalled();
    expect(onRun).not.toHaveBeenCalled();
  });

  it("does not run when disabled", () => {
    const { onRun, button } = setup(null, true);
    fireEvent.click(button);
    expect(onRun).not.toHaveBeenCalled();
  });
});
