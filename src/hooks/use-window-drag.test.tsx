import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWindowDrag } from "./use-window-drag";

const win = vi.hoisted(() => ({ startDragging: vi.fn(), toggleMaximize: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

function Bar() {
  const drag = useWindowDrag();
  return (
    <div data-testid="bar" {...drag}>
      <div data-testid="gap" />
      <button type="button">
        <span data-testid="inside">Save</span>
      </button>
      <div role="slider" aria-valuenow={1} data-testid="slider" />
    </div>
  );
}

describe("useWindowDrag", () => {
  beforeEach(() => {
    win.startDragging.mockClear();
    win.toggleMaximize.mockClear();
  });

  it("drags from empty space between a screen's buttons", () => {
    render(<Bar />);
    fireEvent.mouseDown(screen.getByTestId("gap"), { button: 0 });
    expect(win.startDragging).toHaveBeenCalledTimes(1);
  });

  it("never drags from a button, a child of one, or a slider", () => {
    render(<Bar />);
    fireEvent.mouseDown(screen.getByRole("button"), { button: 0 });
    fireEvent.mouseDown(screen.getByTestId("inside"), { button: 0 });
    fireEvent.mouseDown(screen.getByTestId("slider"), { button: 0 });
    expect(win.startDragging).not.toHaveBeenCalled();
  });

  it("ignores the right button", () => {
    render(<Bar />);
    fireEvent.mouseDown(screen.getByTestId("gap"), { button: 2 });
    expect(win.startDragging).not.toHaveBeenCalled();
  });

  it("maximises on a double click of empty space only", () => {
    render(<Bar />);
    fireEvent.doubleClick(screen.getByTestId("inside"));
    expect(win.toggleMaximize).not.toHaveBeenCalled();
    fireEvent.doubleClick(screen.getByTestId("gap"));
    expect(win.toggleMaximize).toHaveBeenCalledTimes(1);
  });
});
