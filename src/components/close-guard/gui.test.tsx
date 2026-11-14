import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { ConfirmDialogHost } from "@/components/confirm-dialog";
import { CLOSE_REQUESTED } from "@/lib/events";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import { CloseGuard } from "./gui";

function mount() {
  render(
    <Providers>
      <ConfirmDialogHost />
      <CloseGuard />
    </Providers>,
  );
}

describe("CloseGuard", () => {
  it("keeps Lumen open by default when an import is running", async () => {
    mockCommand("confirm_close", () => null);
    mount();
    await waitFor(() => emitMock(CLOSE_REQUESTED, 1));
    const keep = await screen.findByRole("button", { name: "Keep Lumen open" });
    expect(keep).toHaveFocus();
    await userEvent.click(keep);
    await waitFor(() => expect(screen.queryByText("An import is still running")).toBeNull());
    expect(invokeMock).not.toHaveBeenCalledWith("confirm_close", expect.anything());
  });

  it("stops the import and closes when asked", async () => {
    mockCommand("confirm_close", () => null);
    mount();
    await waitFor(() => emitMock(CLOSE_REQUESTED, 1));
    await userEvent.click(await screen.findByRole("button", { name: "Stop import and close" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("confirm_close", undefined));
  });
});
