import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { InlineName } from "./gui";

const setup = () => {
  const onCommit = vi.fn();
  const onCancel = vi.fn();
  render(<InlineName value="Mask 1" editing onCommit={onCommit} onCancel={onCancel} />);
  return { onCommit, onCancel, input: screen.getByRole("textbox") as HTMLInputElement };
};

describe("InlineName", () => {
  it("selects the name on open, then Enter commits the trimmed text", async () => {
    const { onCommit, input } = setup();
    expect(input.selectionStart).toBe(0);
    expect(input.selectionEnd).toBe("Mask 1".length);
    await userEvent.keyboard("  Sky  {Enter}");
    expect(onCommit).toHaveBeenCalledExactlyOnceWith("Sky");
  });

  it("Esc cancels", async () => {
    const { onCommit, onCancel } = setup();
    await userEvent.keyboard("Sky{Escape}");
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("an empty name commits nothing", async () => {
    const { onCommit, onCancel } = setup();
    await userEvent.keyboard("{Backspace}{Enter}");
    expect(onCommit).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("blur commits", async () => {
    const { onCommit, input } = setup();
    await userEvent.keyboard("Sky");
    input.blur();
    expect(onCommit).toHaveBeenCalledExactlyOnceWith("Sky");
  });

  it("shows plain text when not editing", () => {
    render(<InlineName value="Mask 1" editing={false} onCommit={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByText("Mask 1")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).toBeNull();
  });
});
