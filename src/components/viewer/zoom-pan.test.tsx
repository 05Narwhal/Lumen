import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ZoomPan } from "./zoom-pan";

describe("ZoomPan", () => {
  /**
   * Zooming past fit is what makes the viewer swap the 2560 preview for the full-resolution
   * original, so the swap must keep the zoom. It used to reset to fit the moment the sharp
   * file arrived, which is why a zoomed photo only ever looked sharp on the second try.
   */
  it("keeps the zoom when a sharper source replaces the current one", () => {
    const { container, rerender } = render(<ZoomPan src="preview.jpg" alt="" natural={{ width: 4032, height: 3024 }} />);
    const img = container.querySelector("img")!;
    fireEvent.doubleClick(container.querySelector(".zoom-pan")!);
    const zoomed = img.style.transform;
    expect(zoomed).not.toContain("scale(1)");
    rerender(<ZoomPan src="full.jpg" alt="" natural={{ width: 4032, height: 3024 }} />);
    expect(container.querySelector("img")!.style.transform).toBe(zoomed);
  });
});
