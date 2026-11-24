import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginInspect } from "@/types/generated/PluginInspect";
import { neverText, PluginInstallHost } from "./gui";
import { closePluginInstall, openPluginInstall } from "./store";

const info = (over: Partial<PluginInfo> = {}): PluginInfo => ({
  id: "studio.jl.takeout",
  name: "Google Takeout",
  version: "1.0.0",
  author: "JL Studios",
  description: "",
  license: "MIT",
  trust: { type: "jlStudios" },
  kinds: ["importer"],
  permissions: ["importReadChosen"],
  enabled: false,
  dev: false,
  state: "ready",
  iconUrl: null,
  settings: [],
  assets: [],
  ...over,
} as PluginInfo);

const inspect = (over: Partial<PluginInspect> = {}): PluginInspect => ({
  info: info(),
  adds: ['An importer: "Google Takeout"'],
  problems: [],
  notes: [],
  update: null,
  older: null,
  ...over,
});

const installs = () => invokeMock.mock.calls.filter(([c]) => c === "install_plugin").length;

function setup(result: PluginInspect | ((path: string) => PluginInspect), pending: string | null = null) {
  mockCommand("inspect_plugin_package", (a) => (typeof result === "function" ? result(String(a?.path)) : result));
  mockCommand("install_plugin", () => info());
  mockCommand("take_pending_plugin_package", () => pending);
  invokeMock.mockClear();
  return render(
    <Providers>
      <PluginInstallHost />
    </Providers>,
  );
}

describe("PluginInstallHost", () => {
  beforeEach(() => act(() => closePluginInstall()));

  it("installs_a_second_package_after_the_first", async () => {
    setup(inspect());
    act(() => openPluginInstall("C:/a.lumenplugin"));
    await userEvent.click(await screen.findByRole("button", { name: "Install" }));
    await waitFor(() => expect(installs()).toBe(1));
    act(() => openPluginInstall("C:/b.lumenplugin"));
    await userEvent.click(await screen.findByRole("button", { name: "Install" }));
    await waitFor(() => expect(installs()).toBe(2));
  });

  it("renders_spec_sheet", async () => {
    setup(inspect());
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("Google Takeout")).toBeInTheDocument();
    expect(screen.getByText("1.0.0")).toBeInTheDocument();
    expect(screen.getByText("JL Studios · ✓ From JL Studios")).toBeInTheDocument();
    expect(screen.getByText('An importer: "Google Takeout"')).toBeInTheDocument();
    expect(screen.getByText("Read the folder or file you choose when you import with it")).toBeInTheDocument();
    expect(screen.getByText("Touch your originals or your library, use the internet, or start other programs.")).toBeInTheDocument();
    expect(screen.getByText("MIT")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument();
  });

  it("data_only_runs_no_code", async () => {
    setup(inspect({ info: info({ permissions: [], kinds: ["presets"] }) }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("It runs no code.")).toBeInTheDocument();
  });

  it("code_plugin_without_permissions_is_not_codeless", async () => {
    setup(inspect({ info: info({ permissions: [], kinds: ["importer"] }) }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    await screen.findByText("Google Takeout");
    expect(screen.queryByText("It runs no code.")).toBeNull();
  });

  it("trust_lines", async () => {
    let trust: PluginInfo["trust"] = { type: "author", name: "Ada", fingerprint: "A1B2-C3D4" };
    setup(() => inspect({ info: info({ author: "Ada", trust }) }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("Ada · Signed by Ada · key A1B2-C3D4")).toBeInTheDocument();
    trust = { type: "unsigned" };
    act(() => openPluginInstall("C:/b.lumenplugin"));
    expect(await screen.findByText("Ada · ⚠ Unsigned")).toBeInTheDocument();
  });

  it("problems_replace_install", async () => {
    setup(inspect({ problems: ["This plugin needs a newer Lumen."] }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("This plugin needs a newer Lumen.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("update_header_and_new", async () => {
    setup(inspect({ info: info({ version: "1.1.0", permissions: ["importReadChosen", "storage"] }), update: { from: "1.0.0", newPermissions: ["storage"] } }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("Update 1.0.0 → 1.1.0")).toBeInTheDocument();
    expect(screen.getAllByText("New")).toHaveLength(1);
    expect(screen.getByText("Keep its own files (up to 50 MB)").parentElement).toHaveTextContent("New");
  });

  it("older_text", async () => {
    setup(inspect({ older: "1.1.0" }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("Older than the installed 1.1.0")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("notes_listed", async () => {
    setup(inspect({ notes: ["Drops XMP ratings.", "Drops XMP labels."] }));
    act(() => openPluginInstall("C:/a.lumenplugin"));
    expect(await screen.findByText("Drops XMP ratings.")).toBeInTheDocument();
    expect(screen.getByText("Drops XMP labels.")).toBeInTheDocument();
  });

  it("second_open_replaces", async () => {
    setup((p) => inspect({ info: info({ name: p.includes("two") ? "Second" : "First" }) }));
    act(() => openPluginInstall("C:/one.lumenplugin"));
    expect(await screen.findByText("First")).toBeInTheDocument();
    act(() => emitMock("plugins://open-package", "C:/two.lumenplugin"));
    expect(await screen.findByText("Second")).toBeInTheDocument();
    expect(screen.queryByText("First")).toBeNull();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("install_once", async () => {
    setup(inspect());
    act(() => openPluginInstall("C:/a.lumenplugin"));
    const btn = await screen.findByRole("button", { name: "Install" });
    await userEvent.dblClick(btn);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(installs()).toBe(1);
  });

  it("keyboard", async () => {
    setup(inspect());
    act(() => openPluginInstall("C:/a.lumenplugin"));
    const cancel = await screen.findByRole("button", { name: "Cancel" });
    await waitFor(() => expect(cancel).toHaveFocus());
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(installs()).toBe(0);
  });

  it("cold_start_pending", async () => {
    setup(inspect(), "C:/x.lumenplugin");
    expect(await screen.findByText("Google Takeout")).toBeInTheDocument();
    const call = invokeMock.mock.calls.find(([c]) => c === "inspect_plugin_package");
    expect(call?.[1]).toEqual({ path: "C:/x.lumenplugin" });
  });
});

describe("neverText (Round 24)", () => {
  it("narrows for originals.gps and providers", () => {
    expect(neverText([], [])).toBe("Touch your originals or your library, use the internet, or start other programs.");
    expect(neverText(["originalsGps"], ["Open Disk Atlas for file dialogs"])).toBe(
      "Change anything else in your originals or your library, use the internet, or start programs other than the ones above.",
    );
  });
});
