import { act, fireEvent, render, screen } from "@testing-library/react";
import { MantineProvider } from "@mantine/core";
import { afterEach, describe, expect, it } from "vitest";
import { healthStore } from "@/lib/health";
import { HealthBanner } from "./gui";

const show = () => render(<MantineProvider><HealthBanner /></MantineProvider>);

describe("HealthBanner", () => {
  afterEach(() => act(() => healthStore.set({ backend: "ok", stalls: [], busySince: null })));

  it("shows nothing while the backend answers", () => {
    const { container } = show();
    expect(container.querySelector(".health-banner")).toBeNull();
  });

  it("says what is busy, and for how long", () => {
    healthStore.set({ backend: "slow", stalls: [{ what: "database call at a.rs:1", kind: "Database", forMs: 12_000 }], busySince: Date.now() - 40_000 });
    show();
    expect(screen.getByText("Lumen is busy: the library database (40 s).")).toBeInTheDocument();
  });

  it("offers Restart Lumen when stuck, and Wait hides it until the backend recovers", () => {
    healthStore.set({ backend: "stuck", stalls: [], busySince: null });
    show();
    expect(screen.getByText("Lumen is not responding.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart Lumen" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Wait" }));
    expect(screen.queryByText("Lumen is not responding.")).toBeNull();
    act(() => healthStore.set({ backend: "ok", stalls: [], busySince: null }));
    act(() => healthStore.set({ backend: "stuck", stalls: [], busySince: null }));
    expect(screen.getByText("Lumen is not responding.")).toBeInTheDocument();
  });
});
