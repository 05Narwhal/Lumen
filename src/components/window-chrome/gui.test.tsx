import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WindowChrome } from "./gui";

const chrome = () => document.querySelector<HTMLElement>(".window-chrome")!;
const controls = () => document.querySelector<HTMLElement>(".window-chrome__controls")!;

describe("WindowChrome", () => {
  it("is always visible without autoHide", () => {
    render(<WindowChrome />);
    expect(chrome().dataset.visible).toBe("true");
  });

  it("with autoHide shows the controls only while the pointer is in the strip", () => {
    render(<WindowChrome autoHide />);
    expect(chrome().dataset.visible).toBe("false");
    fireEvent.mouseEnter(controls());
    expect(chrome().dataset.visible).toBe("true");
    fireEvent.mouseLeave(controls());
    expect(chrome().dataset.visible).toBe("false");
  });

  it("with autoHide stays visible while a control has focus", () => {
    render(<WindowChrome autoHide />);
    act(() => screen.getByRole("button", { name: "Minimize" }).focus());
    expect(chrome().dataset.visible).toBe("true");
    fireEvent.mouseLeave(controls());
    expect(chrome().dataset.visible).toBe("true");
  });
});
