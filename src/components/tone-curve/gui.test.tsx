import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ToneCurve } from "./gui";
import { addPoint, movePoint, removePoint, withEnds } from "./points";

describe("curve points", () => {
  it("adds the ends a curve implies", () => {
    expect(withEnds([])).toEqual([
      [0, 0],
      [1, 1],
    ]);
    expect(withEnds([[0.5, 0.6]])).toEqual([
      [0, 0],
      [0.5, 0.6],
      [1, 1],
    ]);
  });

  it("adds a point in order, but not on top of another", () => {
    const r = addPoint([], 0.4, 0.5)!;
    expect(r.points).toEqual([
      [0, 0],
      [0.4, 0.5],
      [1, 1],
    ]);
    expect(r.index).toBe(1);
    expect(addPoint(r.points, 0.405, 0.9)).toBeNull();
  });

  it("moves a point only between its neighbours, and the ends only up and down", () => {
    const p = withEnds([
      [0.3, 0.3],
      [0.6, 0.6],
    ]);
    expect(movePoint(p, 1, 0.9, 0.2)[1]).toEqual([0.59, 0.2]);
    expect(movePoint(p, 1, -1, 2)[1]).toEqual([0.01, 1]);
    expect(movePoint(p, 0, 0.5, 0.1)[0]).toEqual([0, 0.1]);
    const moved = movePoint(p, 2, 0.1, 0.5);
    expect(moved.map((q) => q[0])).toEqual([...moved.map((q) => q[0])].sort((a, b) => a - b));
  });

  it("removes a point but never an end", () => {
    const p = withEnds([[0.5, 0.7]]);
    expect(removePoint(p, 1)).toHaveLength(2);
    expect(removePoint(p, 0)).toBe(p);
  });
});

describe("ToneCurve", () => {
  function mount(points: [number, number][] = []) {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(<ToneCurve points={points} channel="master" label="Curve" onChange={onChange} onCommit={onCommit} />);
    const svg = screen.getByRole("group", { name: "Curve" });
    svg.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
    return { svg, onChange, onCommit };
  }

  it("adds a point where it is clicked and records it on release", () => {
    const { svg, onChange, onCommit } = mount();
    fireEvent.pointerDown(svg, { clientX: 25, clientY: 60, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith([
      [0, 0],
      [0.25, 0.4],
      [1, 1],
    ]);
    fireEvent.pointerUp(svg, { clientX: 25, clientY: 60, pointerId: 1 });
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit.mock.calls[0][1]).toBe("Curve");
  });

  it("nudges a focused point with the arrows and deletes it with Delete", () => {
    const { onCommit } = mount([[0.5, 0.5]]);
    const point = screen.getByRole("slider", { name: "Curve point 2" });
    fireEvent.keyDown(point, { key: "ArrowUp", shiftKey: true });
    expect(onCommit.mock.calls[0][0][1][1]).toBeCloseTo(0.5 + 10 / 255);
    fireEvent.keyDown(point, { key: "Delete" });
    expect(onCommit.mock.calls[1][0]).toEqual([
      [0, 0],
      [1, 1],
    ]);
  });
});
