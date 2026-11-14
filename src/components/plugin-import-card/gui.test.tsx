import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useQuery } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { PluginImporter } from "@/types/generated/PluginImporter";
import { PluginImportCards } from "./gui";

const takeout: PluginImporter = { plugin: "takeout", label: "Google Takeout", pick: "file", extensions: ["zip"], hint: "Pick the export zip." };
const fixture: PluginImporter = { plugin: "fixture", label: "Fixture", pick: "folder", extensions: [], hint: "Pick the folder." };

function setup(list: () => PluginImporter[]) {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  mockCommand("list_plugin_importers", list);
  return render(
    <Providers>
      <PluginImportCards />
    </Providers>,
  );
}

describe("PluginImportCards", () => {
  it("card_per_importer", async () => {
    setup(() => [takeout, fixture]);
    expect(await screen.findByText("From Google Takeout")).toBeInTheDocument();
    expect(screen.getByText("From Fixture")).toBeInTheDocument();
    expect(screen.getByText("Pick the export zip.")).toBeInTheDocument();
    expect(screen.getByText("Pick the folder.")).toBeInTheDocument();
  });

  it("file_pick_uses_extensions", async () => {
    setup(() => [takeout]);
    mockCommand("start_plugin_import", () => 1);
    vi.mocked(openDialog).mockResolvedValueOnce("C:/x/takeout.zip");
    await userEvent.click(await screen.findByRole("button", { name: "Choose file…" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ filters: [{ name: "Google Takeout", extensions: ["zip"] }] })));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_plugin_import", { plugin: "takeout", chosen: "C:/x/takeout.zip" }));
  });

  it("folder_pick_uses_directory", async () => {
    setup(() => [fixture]);
    mockCommand("start_plugin_import", () => 1);
    vi.mocked(openDialog).mockResolvedValueOnce("C:/x/dir");
    await userEvent.click(await screen.findByRole("button", { name: "Choose folder…" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ directory: true })));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_plugin_import", { plugin: "fixture", chosen: "C:/x/dir" }));
  });

  it("start_refreshes_history", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    let batches = 0;
    mockCommand("list_import_batches", () => {
      batches += 1;
      return [];
    });
    mockCommand("list_plugin_importers", () => [fixture]);
    mockCommand("start_plugin_import", () => 1);
    // History, as the Import page shows it beside the cards.
    function History() {
      useQuery({ queryKey: queryKeys.batches, queryFn: () => invoke("list_import_batches", { limit: 50 }) });
      return null;
    }
    render(
      <Providers>
        <History />
        <PluginImportCards />
      </Providers>,
    );
    await waitFor(() => expect(batches).toBe(1));
    vi.mocked(openDialog).mockResolvedValueOnce("C:/x/dir");
    await userEvent.click(await screen.findByRole("button", { name: "Choose folder…" }));
    await waitFor(() => expect(batches).toBe(2));
  });

  it("chosen_root_error_shown", async () => {
    setup(() => [fixture]);
    mockCommand("start_plugin_import", () => {
      throw { code: "plugin_chosen_root", message: "That folder is inside a Lumen library." };
    });
    vi.mocked(openDialog).mockResolvedValueOnce("C:/lib");
    await userEvent.click(await screen.findByRole("button", { name: "Choose folder…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That folder is inside a Lumen library.");
  });

  it("hidden_when_none", async () => {
    const { container } = setup(() => []);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_plugin_importers", undefined));
    expect(container.querySelector(".plugin-import-card")).toBeNull();
    expect(screen.queryByText(/^From /)).toBeNull();
  });

  it("refreshes_on_changed", async () => {
    let list = [takeout];
    setup(() => list);
    expect(await screen.findByText("From Google Takeout")).toBeInTheDocument();
    list = [];
    await act(async () => emitMock("plugins://changed", null));
    await waitFor(() => expect(screen.queryByText("From Google Takeout")).toBeNull());
  });
});
