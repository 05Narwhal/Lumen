import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AssetFace } from "@/types/generated/AssetFace";
import type { Recipe } from "@/types/generated/Recipe";
import { FaceBoxes } from "./face-boxes";

const anna: AssetFace = { id: 1, personId: 7, name: "Anna", x: 0.1, y: 0.2, w: 0.2, h: 0.2, manual: false };
const unknown: AssetFace = { id: 2, personId: null, name: null, x: 0.6, y: 0.2, w: 0.2, h: 0.2, manual: false };
const plain: Recipe = { version: 2, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false, develop: null, trim: null };

function mount(over: Partial<React.ComponentProps<typeof FaceBoxes>> = {}) {
  const onPick = vi.fn();
  const onDrawn = vi.fn();
  render(<FaceBoxes faces={[anna, unknown]} recipe={null} aspect={1} show={false} hovered={null} drawing={false} onPick={onPick} onDrawn={onDrawn} {...over} />);
  return { onPick, onDrawn };
}

describe("FaceBoxes", () => {
  it("draws every face with its name when shown, and only the hovered one otherwise", () => {
    mount({ show: true });
    expect(screen.getByRole("button", { name: "Anna: change who this is" })).toHaveStyle({ left: "10%", width: "20%" });
    expect(screen.getByRole("button", { name: "Unknown: tag this face" })).toHaveTextContent("Unknown");
  });

  it("shows only the face whose row is hovered", () => {
    mount({ hovered: 2 });
    expect(screen.queryByRole("button", { name: /Anna/ })).toBeNull();
    expect(screen.getByRole("button", { name: /Unknown/ })).toBeInTheDocument();
  });

  it("picks a face when its box is clicked", async () => {
    const { onPick } = mount({ show: true });
    await userEvent.click(screen.getByRole("button", { name: /Unknown/ }));
    expect(onPick).toHaveBeenCalledWith(unknown);
  });

  it("asks for a face's menu on right-click (Round 4G)", async () => {
    const onMenu = vi.fn();
    mount({ show: true, onMenu });
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByRole("button", { name: /Unknown/ }) });
    expect(onMenu).toHaveBeenCalledWith(unknown, expect.anything());
  });

  it("places boxes through the photo's edit: a crop moves them, and a face cropped away is not drawn", () => {
    mount({ show: true, recipe: { ...plain, crop: { x: 0.5, y: 0, w: 0.5, h: 1 } } });
    expect(screen.queryByRole("button", { name: /Anna/ })).toBeNull();
    const box = screen.getByRole("button", { name: /Unknown/ });
    expect(parseFloat(box.style.left)).toBeCloseTo(20, 6);
    expect(parseFloat(box.style.width)).toBeCloseTo(40, 6);
  });

  it("hands a drawn box back in the unedited photo's coordinates", () => {
    // The photo is shown turned a quarter clockwise: the shown top-right is the unedited top-left.
    const { onDrawn } = mount({ drawing: true, recipe: { ...plain, rotate: 90 }, aspect: 2 });
    const layer = screen.getByTestId("face-draw");
    layer.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 200, right: 100, bottom: 200, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(layer, { clientX: 50, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(layer, { clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerUp(layer, { clientX: 100, clientY: 100, pointerId: 1 });
    expect(onDrawn).toHaveBeenCalledTimes(1);
    const [x, y, w, h] = onDrawn.mock.calls[0][0] as number[];
    [x, y, w, h].forEach((v, i) => expect(v).toBeCloseTo([0, 0, 0.5, 0.5][i], 5));
  });

  it("ignores a click that draws nothing", () => {
    const { onDrawn } = mount({ drawing: true });
    const layer = screen.getByTestId("face-draw");
    layer.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(layer, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.pointerUp(layer, { clientX: 10, clientY: 10, pointerId: 1 });
    expect(onDrawn).not.toHaveBeenCalled();
  });
});
