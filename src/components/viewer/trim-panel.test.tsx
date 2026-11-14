import { MantineProvider } from "@mantine/core";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { cutRange } from "@/lib/trim";
import { cutsOf, TrimPanel, TrimTimeline, type Selection } from "./trim-panel";

const seg = (startMs: number, endMs: number) => ({ start_ms: startMs, end_ms: endMs });

describe("cutsOf", () => {
  it("is what lies between and around the kept segments", () => {
    expect(cutsOf([seg(1000, 3000), seg(5000, 8000)], 10_000)).toEqual([seg(0, 1000), seg(3000, 5000), seg(8000, 10_000)]);
    expect(cutsOf([seg(0, 10_000)], 10_000)).toEqual([]);
  });
});

describe("TrimTimeline", () => {
  function mount(selection: Selection | null = null) {
    const onSeek = vi.fn();
    const onSelect = vi.fn();
    render(<TrimTimeline durationMs={10_000} segments={[seg(0, 10_000)]} selection={selection} timeMs={0} posterUrl="" onSeek={onSeek} onSelect={onSelect} />);
    const track = screen.getByRole("slider", { name: "Timeline" });
    // 1000 px wide, so a pixel is 10 ms.
    track.getBoundingClientRect = () => ({ left: 0, top: 0, width: 1000, height: 56, right: 1000, bottom: 56, x: 0, y: 0, toJSON: () => ({}) });
    return { track, onSeek, onSelect };
  }

  it("seeks on a click", () => {
    const { track, onSeek, onSelect } = mount();
    fireEvent.pointerDown(track, { clientX: 250, pointerId: 1 });
    fireEvent.pointerUp(track, { clientX: 250, pointerId: 1 });
    expect(onSeek).toHaveBeenCalledWith(2500);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("selects on a drag, in either direction", () => {
    const { track, onSelect } = mount();
    fireEvent.pointerDown(track, { clientX: 600, pointerId: 1 });
    fireEvent.pointerMove(track, { clientX: 300, pointerId: 1 });
    fireEvent.pointerUp(track, { clientX: 300, pointerId: 1 });
    expect(onSelect).toHaveBeenLastCalledWith({ a: 3000, b: 6000 });
  });

  it("moves one end of the selection by its handle", () => {
    const { onSelect } = mount({ a: 2000, b: 6000 });
    const end = screen.getByLabelText("Selection end");
    fireEvent.pointerDown(end, { clientX: 600, pointerId: 2 });
    fireEvent.pointerMove(end, { clientX: 800, pointerId: 2 });
    expect(onSelect).toHaveBeenLastCalledWith({ a: 2000, b: 8000 });
  });

  it("hatches the cuts", () => {
    render(<TrimTimeline durationMs={10_000} segments={[seg(0, 3000), seg(5000, 10_000)]} selection={null} timeMs={0} posterUrl="" onSeek={vi.fn()} onSelect={vi.fn()} />);
    const cut = document.querySelector<HTMLElement>(".trim-timeline__cut")!;
    expect(cut.style.left).toBe("30%");
    expect(cut.style.width).toBe("20%");
  });
});

describe("TrimPanel", () => {
  it("cuts the selection into two kept segments, and says what plays", () => {
    // What the trimmer does with the panel's Cut: the selection comes out of the middle.
    const whole = [seg(0, 10_000)];
    const onCut = vi.fn(() => cutRange(whole, 3000, 5000));
    render(
      <MantineProvider>
        <TrimPanel durationMs={10_000} segments={whole} selection={{ a: 3000, b: 5000 }} timeMs={0} onSelect={vi.fn()} onCut={onCut} onKeep={vi.fn()} onReset={vi.fn()} />
      </MantineProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cut selection" }));
    expect(onCut.mock.results[0].value).toEqual([seg(0, 3000), seg(5000, 10_000)]);
    expect(screen.getByRole("button", { name: "Restore full clip" })).toBeDisabled();
  });

  it("disables the selection actions without a selection", () => {
    render(
      <MantineProvider>
        <TrimPanel durationMs={10_000} segments={[seg(0, 4000)]} selection={null} timeMs={0} onSelect={vi.fn()} onCut={vi.fn()} onKeep={vi.fn()} onReset={vi.fn()} />
      </MantineProvider>,
    );
    expect(screen.getByRole("button", { name: "Cut selection" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Restore full clip" })).toBeEnabled();
    expect(screen.getByText("0:04")).toBeInTheDocument();
  });
});
