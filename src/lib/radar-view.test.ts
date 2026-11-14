import { beforeEach, describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { ViewerScope } from "./viewer-scope";
import { noteRadarGrid, noteRadarViewer } from "./radar-view";

const grid = (person: number): ViewerScope => ({ width: 900, rowHeight: 200, gap: 4, scope: { query: { personIds: [person] } } as ViewerScope["scope"], groupBy: "month" });

describe("radar view", () => {
  beforeEach(() => {
    mockCommand("set_face_radar_view", () => null);
    noteRadarViewer(null);
    noteRadarGrid(null);
    invokeMock.mockClear();
  });

  const last = () => invokeMock.mock.calls.filter((c) => c[0] === "set_face_radar_view").at(-1)?.[1];

  it("checks the grid on screen, then the viewer's grid around its photo", () => {
    const people = grid(7);
    noteRadarGrid(people);
    expect(last()).toEqual({ ...people, at: null, open: true });
    noteRadarViewer({ scope: people, at: 42 });
    expect(last()).toEqual({ ...people, at: 42, open: true });
    noteRadarViewer({ scope: null, at: 43 });
    expect(last()).toEqual({ open: false });
    noteRadarViewer(null);
    expect(last()).toEqual({ ...people, at: null, open: true });
  });

  it("sends nothing when nothing changed", () => {
    noteRadarGrid(grid(7));
    const n = invokeMock.mock.calls.length;
    noteRadarGrid(grid(7));
    expect(invokeMock.mock.calls.length).toBe(n);
  });
});
