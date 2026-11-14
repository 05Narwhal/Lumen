import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { ConfirmDialogHost } from "@/components/confirm-dialog";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { TrashRoute } from "./gui";

describe("TrashRoute", () => {
  it("shows the empty state when nothing is trashed", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <MemoryRouter>
          <TrashRoute />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByRole("status")).toHaveTextContent(/Trash is empty/);
  });

  it("empties the trash after confirming", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_layout", () => ({ totalHeight: 200, itemCount: 3, rowHeights: [200], sections: [] }));
    render(
      <Providers>
        <MemoryRouter>
          <TrashRoute />
          <ConfirmDialogHost />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Empty trash" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Empty trash" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("empty_trash", undefined));
  });
});
