import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open } from "@tauri-apps/plugin-dialog";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { LibraryMissingRoute, LibraryProblemRoute } from "./gui";

describe("LibraryProblemRoute", () => {
  const problem = { path: "E:/Lumen Library", code: "library_too_new", message: "This library was last opened by a newer Lumen." };

  it("says why plainly and tries again on the same library", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("open_library", () => ({ id: "x", name: "Travel SSD", path: problem.path, schemaVersion: 2 }));
    render(
      <Providers>
        <LibraryProblemRoute problem={problem} />
      </Providers>,
    );
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("last opened by a newer Lumen");
    expect(alert).toHaveTextContent(problem.path);
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(invokeMock).toHaveBeenCalledWith("open_library", { path: problem.path });
  });

  it("offers the other libraries", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    render(
      <Providers>
        <LibraryProblemRoute problem={problem} />
      </Providers>,
    );
    expect(screen.getByRole("button", { name: "Choose another library" })).toHaveAttribute("aria-expanded", "false");
  });
});

describe("LibraryMissingRoute", () => {
  it("explains the situation and locates the library from a picked folder", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("locate_library", () => ({ id: "x", name: "Travel SSD", path: "F:/Lumen Library", schemaVersion: 2 }));
    vi.mocked(open).mockResolvedValueOnce("F:/Lumen Library");
    render(
      <Providers>
        <LibraryMissingRoute entry={{ id: "x", name: "Travel SSD", path: "E:/Lumen Library", lastOpenedAt: 1 }} />
      </Providers>,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("E:/Lumen Library");
    await userEvent.click(screen.getByRole("button", { name: "Locate…" }));
    expect(invokeMock).toHaveBeenCalledWith("locate_library", { id: "x", path: "F:/Lumen Library" });
  });
});
