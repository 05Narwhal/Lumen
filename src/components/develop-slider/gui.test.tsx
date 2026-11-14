import { act, fireEvent, render, screen } from "@testing-library/react";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DevelopSlider } from "./gui";

const fmt = (v: number) => String(Math.round(v));
const slider = (value: number) => <DevelopSlider label="Exposure" value={value} min={-100} max={100} format={fmt} onChange={vi.fn()} onCommit={vi.fn()} />;
const shown = () => screen.getByText(/^-?\d+$/).textContent;

describe("DevelopSlider", () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance", "setTimeout"] }));
  afterEach(() => vi.useRealTimers());

  it("glides to a value set from outside (another photo opened)", () => {
    const { rerender } = render(<MantineProvider>{slider(0)}</MantineProvider>);
    rerender(<MantineProvider>{slider(80)}</MantineProvider>);
    expect(shown()).toBe("0");
    act(() => vi.advanceTimersByTime(120));
    const mid = Number(shown());
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(80);
    act(() => vi.advanceTimersByTime(400));
    expect(shown()).toBe("80");
  });

  it("follows the pointer at once while it is dragged", () => {
    const { container, rerender } = render(<MantineProvider>{slider(0)}</MantineProvider>);
    fireEvent.pointerDown(container.querySelector(".develop-slider") as Element);
    rerender(<MantineProvider>{slider(35)}</MantineProvider>);
    expect(shown()).toBe("35");
  });
});
