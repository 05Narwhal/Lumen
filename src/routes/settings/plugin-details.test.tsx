import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { setDeveloperMode } from "@/hooks/use-plugins";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { closePluginDetails, openPluginDetails, PluginDetailsHost } from "./plugin-details";

const info = (over: Partial<PluginInfo> = {}): PluginInfo =>
  ({
    id: "studio.jl.takeout",
    name: "Google Takeout",
    version: "1.2.0",
    author: "JL Studios",
    description: "Reads Takeout folders.",
    license: "MIT",
    trust: { type: "jlStudios" },
    kinds: ["importer", "theme"],
    permissions: ["importReadChosen", "storage"],
    enabled: true,
    dev: false,
    state: { type: "ready" },
    iconUrl: null,
    settings: [],
    assets: [{ path: "fonts/a.woff2", license: "OFL-1.1" }],
    ...over,
  }) as PluginInfo;

const calls = (name: string) => invokeMock.mock.calls.filter(([c]) => c === name);

function mount(p: PluginInfo) {
  mockCommand("list_plugins", () => [p]);
  mockCommand("set_plugin_setting", () => undefined);
  mockCommand("set_plugin_developer_mode", () => undefined);
  return render(
    <Providers>
      <PluginDetailsHost />
    </Providers>,
  );
}

const settings: PluginInfo["settings"] = [
  { key: "on", label: "Switch it", kind: "bool", value: false, choices: [] },
  { key: "n", label: "Count", kind: "number", value: 3, choices: [] },
  { key: "t", label: "Name", kind: "text", value: "a", choices: [] },
  { key: "c", label: "Style", kind: "choice", value: "one", choices: ["one", "two"] },
];

describe("plugin details", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    closePluginDetails();
    mockCommand("set_plugin_developer_mode", () => undefined);
    await setDeveloperMode(false);
  });

  it("details_lists_adds_permissions_licence", async () => {
    mount(info());
    openPluginDetails("studio.jl.takeout", "details");
    expect(await screen.findByText("Adds an importer")).toBeInTheDocument();
    expect(screen.getByText("Adds a theme")).toBeInTheDocument();
    expect(screen.getByText(/Read the folder or file you choose/)).toBeInTheDocument();
    expect(screen.getByText(/Keep its own files/)).toBeInTheDocument();
    expect(screen.getByText("MIT")).toBeInTheDocument();
    expect(screen.getByText(/fonts\/a\.woff2/)).toBeInTheDocument();
    expect(screen.getByText(/OFL-1\.1/)).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Log" })).toBeNull();
  });

  it("settings_form_controls", async () => {
    const user = userEvent.setup();
    mount(info({ settings }));
    openPluginDetails("studio.jl.takeout", "settings");
    await user.click(await screen.findByRole("switch", { name: "Switch it" }));
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "studio.jl.takeout", key: "on", value: true }));
    const n = screen.getByLabelText("Count");
    await user.clear(n);
    await user.type(n, "7");
    await user.tab();
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "studio.jl.takeout", key: "n", value: 7 }));
    await user.type(screen.getByLabelText("Name"), "b");
    await user.tab();
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "studio.jl.takeout", key: "t", value: "ab" }));
    await user.click(screen.getByRole("combobox", { name: "Style" }));
    await user.click(await screen.findByRole("option", { name: "two" }));
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "studio.jl.takeout", key: "c", value: "two" }));
  });

  it("number_setting_validated", async () => {
    const user = userEvent.setup();
    mount(info({ settings }));
    openPluginDetails("studio.jl.takeout", "settings");
    const n = await screen.findByLabelText("Count");
    await user.clear(n);
    await user.type(n, "abc");
    expect(await screen.findByText("Enter a number")).toBeInTheDocument();
    expect(calls("set_plugin_setting")).toHaveLength(0);
  });

  it("log_shows_last_lines", async () => {
    await setDeveloperMode(true);
    mockCommand("plugin_log", () => Array.from({ length: 200 }, (_, i) => `line ${i}`));
    mount(info());
    openPluginDetails("studio.jl.takeout", "log");
    const log = await screen.findByRole("log");
    expect(log.children).toHaveLength(200);
    expect(screen.getByText("line 199")).toBeInTheDocument();
    expect(calls("plugin_log")[0][1]).toEqual({ id: "studio.jl.takeout" });
  });
});

describe("plugin details fields", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    closePluginDetails();
    mockCommand("set_plugin_developer_mode", () => undefined);
    await setDeveloperMode(false);
  });

  it("switching plugin shows the new value for the same key", async () => {
    const a = info({ id: "a", name: "A", settings: [{ key: "t", label: "Name", kind: "text", value: "alpha", choices: [] }] });
    const b = info({ id: "b", name: "B", settings: [{ key: "t", label: "Name", kind: "text", value: "beta", choices: [] }] });
    mockCommand("list_plugins", () => [a, b]);
    render(
      <Providers>
        <PluginDetailsHost />
      </Providers>,
    );
    openPluginDetails("a", "settings");
    expect(await screen.findByDisplayValue("alpha")).toBeInTheDocument();
    openPluginDetails("b", "settings");
    expect(await screen.findByDisplayValue("beta")).toBeInTheDocument();
  });

  it("a refetched value updates the field", async () => {
    let v = "one";
    mockCommand("list_plugins", () => [info({ settings: [{ key: "t", label: "Name", kind: "text", value: v, choices: [] }] })]);
    render(
      <Providers>
        <PluginDetailsHost />
      </Providers>,
    );
    openPluginDetails("studio.jl.takeout", "settings");
    expect(await screen.findByDisplayValue("one")).toBeInTheDocument();
    v = "two";
    await act(async () => emitMock("plugins://changed", null));
    expect(await screen.findByDisplayValue("two")).toBeInTheDocument();
  });

  it("typing then blur saves once", async () => {
    const user = userEvent.setup();
    mockCommand("set_plugin_setting", () => undefined);
    mockCommand("list_plugins", () => [info({ settings: [{ key: "t", label: "Name", kind: "text", value: "a", choices: [] }] })]);
    render(
      <Providers>
        <PluginDetailsHost />
      </Providers>,
    );
    openPluginDetails("studio.jl.takeout", "settings");
    await user.type(await screen.findByLabelText("Name"), "bc");
    expect(calls("set_plugin_setting")).toHaveLength(0);
    await user.tab();
    await waitFor(() => expect(calls("set_plugin_setting")).toHaveLength(1));
    expect(calls("set_plugin_setting")[0][1]).toEqual({ id: "studio.jl.takeout", key: "t", value: "abc" });
  });
});
