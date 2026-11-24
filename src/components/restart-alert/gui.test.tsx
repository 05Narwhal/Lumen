import { MantineProvider } from "@mantine/core";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RestartAlert } from "./gui";

const show = (route: string | null, onBack = vi.fn(), onDismiss = vi.fn()) =>
  render(
    <MantineProvider>
      <RestartAlert route={route} onBack={onBack} onDismiss={onDismiss} />
    </MantineProvider>,
  );

describe("RestartAlert", () => {
  it("says the edits are kept and takes the person back to Develop", () => {
    const onBack = vi.fn();
    show("/develop/12?panel=detail", onBack);
    expect(screen.getByRole("alert")).toHaveTextContent("Lumen restarted after a problem. Your edits are kept.");
    fireEvent.click(screen.getByRole("button", { name: "Back to Develop" }));
    expect(onBack).toHaveBeenCalledWith("/develop/12?panel=detail");
  });

  it("offers no way back when the person was not in Develop", () => {
    const onDismiss = vi.fn();
    show("/library", vi.fn(), onDismiss);
    expect(screen.queryByRole("button", { name: "Back to Develop" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onDismiss).toHaveBeenCalled();
  });
});
