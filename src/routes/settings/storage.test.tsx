import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { Toaster } from "@/components/toast";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { StorageSection } from "./storage";

const GB = 1024 ** 3;

function usage(over: { warn?: boolean; total?: number; rows?: object[] } = {}) {
  const rows = over.rows ?? [
    { kind: "keptAside", bytes: 3 * GB, files: 12 },
    { kind: "unfinished", bytes: 5_000_000, files: 2 },
    { kind: "dngCache", bytes: 40_000_000, files: 1 },
  ];
  return { rows, total: over.total ?? 3 * GB + 45_000_000, warn: over.warn ?? true };
}

function mount(temp: object, clear?: (a?: Record<string, unknown>) => unknown) {
  mockCommand("storage_overview", () => ({ disks: [], libraryPath: "C:/Lumen Library", library: [], app: [] }));
  mockCommand("get_temp_usage", () => temp);
  if (clear) mockCommand("clear_temp", clear);
  return render(
    <Providers>
      <MemoryRouter>
        <StorageSection />
        <Toaster />
      </MemoryRouter>
    </Providers>,
  );
}

describe("Storage: temporary files", () => {
  it("shows one ticked row per kind that holds something, and the warning above the limit", async () => {
    mount(usage());
    expect(await screen.findByText(/Temporary files · /)).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Replaced by Repair" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Unfinished copies" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Conversion cache" })).toBeChecked();
    expect(screen.queryByRole("checkbox", { name: "Recoveries not kept" })).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent(/Temporary files use 3(\.\d)? GB\./);
  });

  it("has no warning below the limit and no group when nothing is there", async () => {
    mount(usage({ warn: false }));
    await screen.findByText(/Temporary files · /);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("leaves the group out when there are no temporary files", async () => {
    mount({ rows: [], total: 0, warn: false });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_temp_usage", undefined));
    await screen.findByText("Disks");
    expect(screen.queryByText(/Temporary files/)).toBeNull();
  });

  it("clears only the ticked kinds and says how much was freed", async () => {
    const seen: unknown[] = [];
    mount(usage({ warn: false }), (a) => {
      seen.push(a?.kinds);
      return 45_000_000;
    });
    const u = userEvent.setup();
    await u.click(await screen.findByRole("checkbox", { name: "Replaced by Repair" }));
    await u.click(screen.getByRole("button", { name: "Clear temporary files" }));
    await waitFor(() => expect(seen).toEqual([["unfinished", "dngCache"]]));
    expect(await screen.findByText("Freed 45 MB.")).toBeInTheDocument();
  });

  it("points at the Recycle Bin when the replaced originals went", async () => {
    mount(usage({ warn: false }), () => 3 * GB);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Clear temporary files" }));
    expect(await screen.findByText(/The replaced originals are in the Recycle Bin\./)).toBeInTheDocument();
  });

  it("disables the button with nothing ticked", async () => {
    mount(usage({ warn: false, rows: [{ kind: "dngCache", bytes: 40_000_000, files: 1 }] }));
    const u = userEvent.setup();
    await u.click(await screen.findByRole("checkbox", { name: "Conversion cache" }));
    expect(screen.getByRole("button", { name: "Clear temporary files" })).toBeDisabled();
  });

  it("says to wait while a repair runs", async () => {
    mount(usage({ warn: false }), () => {
      throw { code: "busy", message: "Wait for Repair to finish.", detail: null };
    });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Clear temporary files" }));
    expect(await screen.findByText("Wait for Repair to finish.")).toBeInTheDocument();
  });
});
