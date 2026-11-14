import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MantineProvider } from "@mantine/core";
import { afterEach, describe, expect, it } from "vitest";
import { jobsStore, removeJob, upsertJob } from "@/lib/jobs-store";
import { ActivityRing } from "./gui";

const mount = () =>
  render(
    <MantineProvider env="test">
      <ActivityRing />
    </MantineProvider>,
  );

describe("ActivityRing (Round 9B)", () => {
  afterEach(() => {
    for (const j of jobsStore.get().jobs) removeJob(j.id);
  });

  it("is idle with nothing running", () => {
    mount();
    expect(screen.getByRole("button", { name: "Nothing running" })).toHaveAttribute("data-active", "false");
  });

  it("names the work and shows plain counts, never workers", async () => {
    mount();
    act(() => {
      upsertJob({ id: "people", kind: "maintenance", label: "Looking for people", progress: 0.25, done: 250, total: 1000, state: "running", detail: "750 left" });
      upsertJob({ id: "activity:video-prep", kind: "maintenance", label: "Preparing videos", progress: 0.5, done: 2, total: 4, state: "running" });
    });
    const ring = screen.getByRole("button", { name: /Looking for people, 25% done/ });
    await userEvent.click(ring);
    expect(await screen.findByText("750 left")).toBeInTheDocument();
    expect(screen.getByText("2 of 4")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/worker/i);
  });

  it("draws everything done over everything there is", () => {
    mount();
    act(() => upsertJob({ id: "a", kind: "maintenance", label: "A", progress: 0.9, done: 90, total: 100, state: "running" }));
    const fill = screen.getByTestId("activity-ring-fill");
    const circumference = Number(fill.getAttribute("stroke-dasharray"));
    expect(Number(fill.getAttribute("stroke-dashoffset")) / circumference).toBeCloseTo(0.1);
    act(() => upsertJob({ id: "b", kind: "import", label: "Importing 4 items", progress: 0, done: 0, total: 4, state: "running" }));
    // Not halved by a line that just started.
    expect(1 - Number(fill.getAttribute("stroke-dashoffset")) / circumference).toBeCloseTo(90 / 104);
  });
});
