import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Section } from "@/types/generated/Section";
import { TimelineScrubber } from "./gui";

const section = (index: number, year: number, month: number, y: number, title: string): Section => ({
  index,
  year,
  month,
  title,
  y,
  height: 0,
  firstRow: index,
  rowCount: 1,
  itemCount: 1,
  albumId: null,
});

const SECTIONS = [section(0, 2024, 5, 0, "May 2024 · Lisbon"), section(1, 2023, 12, 6000, "December 2023")];
const TOTAL = 10_000;
const VIEW = 800;
const TRACK = 400;

/** A scroller with a size, since jsdom lays nothing out. */
function scroller(scrollTop = 0) {
  const el = document.createElement("div");
  Object.defineProperty(el, "clientHeight", { value: VIEW, configurable: true });
  el.scrollTop = scrollTop;
  return el;
}

beforeEach(() => {
  // The track is TRACK px tall from the top of the window.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    top: 0,
    left: 0,
    right: 20,
    bottom: TRACK,
    width: 20,
    height: TRACK,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
});
afterEach(() => vi.restoreAllMocks());

function renderScrubber(props: Partial<Parameters<typeof TimelineScrubber>[0]> = {}) {
  const onScrollTo = vi.fn();
  const el = props.scroller ?? scroller();
  render(<TimelineScrubber scroller={el} sections={SECTIONS} total={TOTAL} grouping="month" onScrollTo={onScrollTo} {...props} />);
  return { onScrollTo, el };
}

describe("TimelineScrubber", () => {
  it("is a scrollbar whose value is the date in view", () => {
    renderScrubber();
    const bar = screen.getByRole("scrollbar");
    expect(bar).toHaveAttribute("aria-orientation", "vertical");
    expect(bar).toHaveAttribute("aria-valuetext", "May 2024");
  });

  it("dragging the track scrolls the grid to the matching offset", () => {
    const { onScrollTo } = renderScrubber();
    const bar = screen.getByRole("scrollbar");
    fireEvent.pointerDown(bar, { clientY: 200, pointerId: 1, buttons: 1 });
    expect(onScrollTo).toHaveBeenLastCalledWith((TOTAL - VIEW) / 2);
    fireEvent.pointerMove(bar, { clientY: 400, pointerId: 1, buttons: 1 });
    expect(onScrollTo).toHaveBeenLastCalledWith(TOTAL - VIEW);
    fireEvent.pointerUp(bar, { clientY: 400, pointerId: 1 });
    onScrollTo.mockClear();
    fireEvent.pointerMove(bar, { clientY: 100, pointerId: 1, buttons: 0 });
    expect(onScrollTo).not.toHaveBeenCalled();
  });

  it("tells the grid when a drag starts and ends, so tiles can hold their images", () => {
    const onDraggingChange = vi.fn();
    renderScrubber({ onDraggingChange });
    const bar = screen.getByRole("scrollbar");
    fireEvent.pointerDown(bar, { clientY: 10, pointerId: 1, buttons: 1 });
    expect(onDraggingChange).toHaveBeenLastCalledWith(true);
    fireEvent.pointerUp(bar, { clientY: 10, pointerId: 1 });
    expect(onDraggingChange).toHaveBeenLastCalledWith(false);
  });

  it("reaches the very bottom, padding under the last row included", () => {
    const el = scroller();
    Object.defineProperty(el, "scrollHeight", { value: TOTAL + 48, configurable: true });
    const { onScrollTo } = renderScrubber({ scroller: el });
    const bar = screen.getByRole("scrollbar");
    fireEvent.pointerDown(bar, { clientY: TRACK + 30, pointerId: 1, buttons: 1 });
    expect(onScrollTo).toHaveBeenLastCalledWith(TOTAL + 48 - VIEW);
  });

  it("hovering shows the date under the pointer in a bubble", () => {
    renderScrubber();
    // 300 px down a 400 px track is 6,900 px into the content: December 2023.
    fireEvent.pointerMove(screen.getByRole("scrollbar"), { clientY: 300, pointerId: 1, buttons: 0 });
    expect(screen.getByText("December 2023")).toHaveClass("timeline-scrubber__bubble");
  });

  it("keeps its year labels, which show only on hover, out of the Tab order", () => {
    renderScrubber();
    expect(screen.getByRole("button", { name: "2023" })).toHaveAttribute("tabindex", "-1");
  });

  it("clicking a year label jumps to that year", () => {
    const { onScrollTo } = renderScrubber();
    fireEvent.click(screen.getByRole("button", { name: "2023" }));
    expect(onScrollTo).toHaveBeenCalledWith(6000);
  });

  it("follows the grid's own scrolling", () => {
    const el = scroller();
    renderScrubber({ scroller: el });
    el.scrollTop = 6500;
    fireEvent.scroll(el);
    expect(screen.getByRole("scrollbar")).toHaveAttribute("aria-valuetext", "December 2023");
  });

  it("hides when everything fits in the view", () => {
    renderScrubber({ total: VIEW - 10 });
    expect(screen.queryByRole("scrollbar")).not.toBeInTheDocument();
  });

  it("an album in manual order gets a plain track with no labels or dates", () => {
    renderScrubber({ grouping: null, sections: [section(0, 2024, 5, 0, "")] });
    const bar = screen.getByRole("scrollbar");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(bar).not.toHaveAttribute("aria-valuetext");
    fireEvent.pointerMove(bar, { clientY: 300, pointerId: 1, buttons: 0 });
    expect(document.querySelector(".timeline-scrubber__bubble")).toBeNull();
  });
});
