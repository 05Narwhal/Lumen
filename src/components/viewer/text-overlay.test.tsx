import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TextOverlay } from "./text-overlay";

const lines = [
  { ord: 0, text: "Gostilna", quad: [0.1, 0.1, 0.5, 0.1, 0.5, 0.2, 0.1, 0.2] as [number, number, number, number, number, number, number, number] },
  // Turned 90°: reads downwards.
  { ord: 1, text: "Kosilo", quad: [0.8, 0.1, 0.8, 0.5, 0.7, 0.5, 0.7, 0.1] as [number, number, number, number, number, number, number, number] },
];

describe("TextOverlay", () => {
  it("draws every line as real text fitted to its quad", () => {
    const { container } = render(<TextOverlay lines={lines} aspect={2} onMenu={() => {}} />);
    const texts = container.querySelectorAll("text");
    expect([...texts].map((t) => t.textContent)).toEqual(["Gostilna", "Kosilo"]);
    // 2:1 photo, 2000 × 1000 units: the first line is 0.4 × 2000 wide, 0.1 × 1000 high.
    expect(Number(texts[0].getAttribute("textLength"))).toBeCloseTo(800);
    expect(texts[0].getAttribute("lengthAdjust")).toBe("spacingAndGlyphs");
    expect(texts[0].getAttribute("transform")).toBe("rotate(0 200 100)");
    expect(texts[1].getAttribute("transform")).toBe("rotate(90 1600 100)");
    expect(container.querySelectorAll(".text-overlay__line")).toHaveLength(2);
  });

  it("hands the line's text to the menu, or the selection when there is one", () => {
    const onMenu = vi.fn();
    const { container } = render(<TextOverlay lines={lines} aspect={2} onMenu={onMenu} />);
    fireEvent.contextMenu(container.querySelectorAll("text")[1]);
    expect(onMenu).toHaveBeenLastCalledWith("Kosilo", expect.anything());
    vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "Gost" } as Selection);
    fireEvent.contextMenu(container.querySelectorAll("text")[1]);
    expect(onMenu).toHaveBeenLastCalledWith("Gost", expect.anything());
    vi.restoreAllMocks();
  });

  it("keeps a drag on the text away from the zoom and pan", () => {
    const parent = vi.fn();
    const { container } = render(
      <div onPointerDown={parent}>
        <TextOverlay lines={lines} aspect={2} onMenu={() => {}} />
      </div>,
    );
    fireEvent.pointerDown(container.querySelector("text")!);
    expect(parent).not.toHaveBeenCalled();
  });
});
