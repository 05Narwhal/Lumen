import { MantineProvider } from "@mantine/core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SplitButton } from "./gui";

describe("SplitButton", () => {
  it("runs the main action on the left and the second entry from the caret's menu", async () => {
    const main = vi.fn();
    const second = vi.fn();
    render(
      <MantineProvider>
        <SplitButton label="New album" onClick={main} menu={[{ label: "New smart album", onClick: second }]} />
      </MantineProvider>,
    );
    await userEvent.click(screen.getByRole("button", { name: "New album" }));
    expect(main).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "More ways to create" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "New smart album" }));
    expect(second).toHaveBeenCalledTimes(1);
  });
});
