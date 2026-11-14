import { MantineProvider } from "@mantine/core";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IDENTITY, type EditSession } from "@/hooks/use-edit-session";
import type { Recipe } from "@/types/generated/Recipe";
import { EditToolbar } from "./edit-toolbar";

function session(draft: Recipe = IDENTITY) {
  let current = draft;
  const s: EditSession = {
    draft,
    saved: null,
    loaded: true,
    previewUrl: null,
    rendering: false,
    dirty: false,
    set: vi.fn((patch: Partial<Recipe> | ((d: Recipe) => Recipe)) => {
      current = typeof patch === "function" ? patch(current) : { ...current, ...patch };
    }),
    reset: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    canUndo: false,
    canRedo: false,
    done: vi.fn(),
    cancel: vi.fn(),
    revert: vi.fn(),
  };
  return { s, now: () => current };
}

function mount(s: EditSession, over: Partial<Parameters<typeof EditToolbar>[0]> = {}) {
  const props = {
    session: s,
    cropping: false,
    onCropping: vi.fn(),
    aspect: null,
    onAspect: vi.fn(),
    grid: "auto" as const,
    onGrid: vi.fn(),
    onStraightening: vi.fn(),
    cropSize: null,
    ...over,
  };
  render(
    <MantineProvider>
      <EditToolbar {...props} />
    </MantineProvider>,
  );
  return props;
}

describe("EditToolbar", () => {
  it("turns and flips what is on screen", () => {
    const { s, now } = session({ ...IDENTITY, rotate: 270 });
    mount(s);
    fireEvent.click(screen.getByRole("button", { name: "Rotate right" }));
    expect(now().rotate).toBe(0);
    fireEvent.click(screen.getByRole("button", { name: "Flip horizontal" }));
    expect(now().flip_h).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Flip vertical" }));
    expect(now().flip_v).toBe(true);
  });

  it("takes an exact straighten angle, clamped, as one undo step with the slider", () => {
    const { s, now } = session();
    mount(s);
    fireEvent.change(screen.getByLabelText("Straighten angle in degrees"), { target: { value: "2.35" } });
    expect(now().straighten).toBe(2.4);
    expect(s.set).toHaveBeenLastCalledWith({ straighten: 2.4 }, "straighten");
    fireEvent.change(screen.getByLabelText("Straighten angle in degrees"), { target: { value: "90" } });
    expect(now().straighten).toBe(45);
  });

  it("is geometry only: no light or colour controls", () => {
    const { s } = session();
    mount(s);
    for (const name of ["Exposure", "Contrast", "Temperature", "Saturation"]) expect(screen.queryByText(name)).not.toBeInTheDocument();
  });

  it("shows crop shapes and the crop's size only while cropping", () => {
    const { s } = session({ ...IDENTITY, crop: { x: 0, y: 0, w: 0.5, h: 0.5 } });
    const props = mount(s, { cropping: true, cropSize: { width: 2016, height: 1512 } });
    expect(screen.getByLabelText("Crop size")).toHaveTextContent("2016 × 1512");
    fireEvent.click(screen.getByRole("radio", { name: "1:1" }));
    expect(props.onAspect).toHaveBeenCalledWith(1);
    fireEvent.click(screen.getByRole("button", { name: "Reset crop" }));
    expect(s.set).toHaveBeenCalledWith({ crop: null });
    fireEvent.click(screen.getByRole("button", { name: "Done cropping" }));
    expect(props.onCropping).toHaveBeenCalledWith(false);
  });

  it("cycles the grid", () => {
    const { s } = session();
    const props = mount(s, { grid: "thirds" });
    fireEvent.click(screen.getByRole("button", { name: "Grid: Thirds" }));
    expect(props.onGrid).toHaveBeenCalledWith("fine");
  });
});
