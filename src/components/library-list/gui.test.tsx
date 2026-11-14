import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { LibraryList } from "./gui";

const listing = {
  libraries: [
    { entry: { id: "a", name: "Lumen Library", path: "C:/Pics/Lumen Library", lastOpenedAt: 3 }, available: true },
    { entry: { id: "b", name: "Archive", path: "D:/Archive", lastOpenedAt: 2 }, available: true },
    { entry: { id: "c", name: "Travel SSD", path: "E:/Lumen Library", lastOpenedAt: 1 }, available: false },
  ],
  current: "a",
};

describe("LibraryList", () => {
  it("renders the three states and forgets a library", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_libraries", () => listing);
    mockCommand("get_current_library", () => ({ id: "a", name: "Lumen Library", path: "C:/Pics/Lumen Library", schemaVersion: 2 }));
    mockCommand("forget_library", () => ({ ...listing, libraries: listing.libraries.slice(0, 2) }));
    render(
      <Providers>
        <LibraryList />
      </Providers>,
    );
    expect(await screen.findByText("Not connected")).toBeInTheDocument();
    expect(screen.getByText("Available")).toBeInTheDocument();
    expect(screen.getAllByText("Open")).toHaveLength(2); // the pill on the current row + the button on the available row
    expect(screen.getByRole("button", { name: "Locate…" })).toBeInTheDocument();
    const forgets = screen.getAllByRole("button", { name: "Forget" });
    expect(forgets).toHaveLength(2);
    await userEvent.click(forgets[1]);
    expect(invokeMock).toHaveBeenCalledWith("forget_library", { id: "c" });
  });
});
