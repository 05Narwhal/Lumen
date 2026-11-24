import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginControl } from "@/types/generated/PluginControl";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { PluginPage } from "./gui";

const ctl = (kind: PluginControl["kind"], key: string | null, label: string, over: Partial<PluginControl> = {}): PluginControl => ({
  kind, key, label, help: null, value: null, choices: [], min: null, max: null, step: null, requires: null, app: null, action: null, body: null, ...over,
});

const plugin = (controls: PluginControl[], over: Partial<PluginInfo> = {}): PluginInfo =>
  ({
    id: "local.sampler", name: "Sampler", version: "1.0.0", author: "Me", description: "Reads sizes.", license: "MIT",
    trust: { type: "jlStudios" }, kinds: ["tools"], permissions: ["storage"], enabled: true, dev: false, state: { type: "ready" },
    iconUrl: null, settings: [], sections: [{ title: "Scanning", description: "How a scan runs.", controls }],
    homepage: null, authorUrl: null, repository: null, screenshots: [], hasReadme: false, detectIds: [], assets: [], ...over,
  }) as PluginInfo;

const calls = (name: string) => invokeMock.mock.calls.filter(([c]) => c === name);

function show(p: PluginInfo, onBack = () => {}) {
  mockCommand("list_plugins", () => [p]);
  mockCommand("set_plugin_setting", () => undefined);
  mockCommand("get_plugin_developer_mode", () => false);
  return render(
    <Providers>
      <MemoryRouter>
        <PluginPage plugin={p} onBack={onBack} reload={() => {}} />
      </MemoryRouter>
    </Providers>,
  );
}

describe("plugin page controls", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("plugin_detect", () => []);
  });

  it("renders the section title, description and each value control, saving changes", async () => {
    const user = userEvent.setup();
    show(
      plugin([
        ctl("toggle", "deep", "Deep scan", { value: false, help: "Reads every file." }),
        ctl("number", "limit", "Largest files", { value: 50 }),
        ctl("text", "label", "Report title", { value: "a" }),
        ctl("select", "order", "Order", { value: "name", choices: ["name", "size"] }),
        ctl("segmented", "unit", "Units", { value: "mb", choices: ["mb", "gb"] }),
        ctl("slider", "depth", "Depth", { value: 3, min: 1, max: 8, step: 1 }),
      ]),
    );
    expect(screen.getByText("Scanning")).toBeInTheDocument();
    expect(screen.getByText("How a scan runs.")).toBeInTheDocument();
    expect(screen.getByText("Reads every file.")).toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Deep scan" }));
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "deep", value: true }));
    const n = screen.getByLabelText("Largest files");
    await user.clear(n);
    await user.type(n, "7");
    await user.tab();
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "limit", value: 7 }));
    await user.type(screen.getByLabelText("Report title"), "b");
    await user.tab();
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "label", value: "ab" }));
    await user.click(screen.getByRole("combobox", { name: "Order" }));
    await user.click(await screen.findByRole("option", { name: "size" }));
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "order", value: "size" }));
    await user.click(screen.getByText("gb"));
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "unit", value: "gb" }));
    const slider = screen.getByRole("slider");
    expect(slider).toHaveAttribute("aria-valuenow", "3");
    slider.focus();
    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(calls("set_plugin_setting").at(-1)?.[1]).toEqual({ id: "local.sampler", key: "depth", value: 4 }));
  });

  it("shows a control with requires only while that control is on or non-empty", () => {
    show(plugin([ctl("toggle", "deep", "Deep scan", { value: false }), ctl("slider", "depth", "Depth", { value: 3, min: 1, max: 8, step: 1, requires: "deep" })]));
    expect(screen.queryByText("Depth")).toBeNull();
  });

  it("shows it when the required toggle is on", () => {
    show(plugin([ctl("toggle", "deep", "Deep scan", { value: true }), ctl("slider", "depth", "Depth", { value: 3, min: 1, max: 8, step: 1, requires: "deep" })]));
    expect(screen.getByText("Depth")).toBeInTheDocument();
  });

  it("disables a control whose app isn't found and offers Locate", async () => {
    mockCommand("plugin_detect", () => [{ id: "atlas", name: "Atlas", path: null, version: null, located: false }]);
    show(plugin([ctl("toggle", "pick", "Use Atlas", { value: true, app: "atlas" })], { detectIds: ["atlas"] }));
    expect(await screen.findByText("Atlas not found")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Locate…" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use Atlas" })).toBeDisabled();
  });

  it("a button runs the action and shows the line it returns", async () => {
    const user = userEvent.setup();
    mockCommand("plugin_ui_action", () => "Scanned 12 folders.");
    show(plugin([ctl("button", null, "Scan now", { action: "scan" })]));
    await user.click(screen.getByRole("button", { name: "Scan now" }));
    expect(await screen.findByText("Scanned 12 folders.")).toBeInTheDocument();
    expect(calls("plugin_ui_action")[0][1]).toEqual({ id: "local.sampler", action: "scan" });
  });

  it("a button that fails shows its message instead", async () => {
    const user = userEvent.setup();
    mockCommand("plugin_ui_action", () => {
      throw { code: "plugin_error", message: "Nothing to scan." };
    });
    show(plugin([ctl("button", null, "Scan now", { action: "scan" })]));
    await user.click(screen.getByRole("button", { name: "Scan now" }));
    expect(await screen.findByText("Nothing to scan.")).toBeInTheDocument();
  });

  it("stat and list show what the plugin returns", async () => {
    mockCommand("plugin_ui_value", (a) => ((a as { key: string }).key === "last" ? "3 minutes ago" : JSON.stringify(["Photos", { text: "Videos", detail: "4 GB" }])));
    show(plugin([ctl("stat", "last", "Last scan"), ctl("list", "big", "Largest folders")]));
    expect(await screen.findByText("3 minutes ago")).toBeInTheDocument();
    expect(await screen.findByText("Photos")).toBeInTheDocument();
    expect(screen.getByText("Videos")).toBeInTheDocument();
    expect(screen.getByText("4 GB")).toBeInTheDocument();
  });

  it("stat and list stay empty when the plugin is off, without calling it", async () => {
    mockCommand("plugin_ui_value", () => "x");
    show(plugin([ctl("stat", "last", "Last scan"), ctl("list", "big", "Largest folders")], { enabled: false }));
    expect(screen.getByText("Last scan")).toBeInTheDocument();
    expect(screen.getByText("Largest folders")).toBeInTheDocument();
    await act(async () => {});
    expect(calls("plugin_ui_value")).toHaveLength(0);
  });

  it("stat and list stay empty when the call errors", async () => {
    mockCommand("plugin_ui_value", () => {
      throw { code: "plugin_off", message: "off" };
    });
    show(plugin([ctl("stat", "last", "Last scan")]));
    await waitFor(() => expect(calls("plugin_ui_value")).toHaveLength(1));
    expect(screen.queryByText("off")).toBeNull();
  });

  it("renders a markdown control", () => {
    show(plugin([ctl("markdown", null, "About", { body: "Reads sizes only. **No file is changed.**" })]));
    expect(screen.getByText("No file is changed.").tagName).toBe("STRONG");
  });

  it("a refetched value updates the field", async () => {
    let v = "one";
    const p = () => plugin([ctl("text", "label", "Report title", { value: v })]);
    mockCommand("list_plugins", () => [p()]);
    const { rerender } = show(p());
    expect(screen.getByDisplayValue("one")).toBeInTheDocument();
    v = "two";
    await act(async () => emitMock("plugins://changed", null));
    rerender(
      <Providers>
        <MemoryRouter>
          <PluginPage plugin={p()} onBack={() => {}} reload={() => {}} />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByDisplayValue("two")).toBeInTheDocument();
  });
});

describe("plugin page details", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("plugin_detect", () => []);
  });

  it("lists what it adds, permissions and licence, and a Remove button that returns", async () => {
    const user = userEvent.setup();
    let back = 0;
    mockCommand("remove_plugin", () => "tok-1");
    show(plugin([]), () => back++);
    expect(screen.getByText("Adds tools")).toBeInTheDocument();
    expect(screen.getByText(/Keep its own files/)).toBeInTheDocument();
    expect(screen.getByText("MIT")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove Sampler" }));
    await waitFor(() => expect(calls("remove_plugin")).toHaveLength(1));
    await waitFor(() => expect(back).toBe(1));
  });

  it("says so when a plugin has no settings", () => {
    show(plugin([], { sections: [] }));
    expect(screen.getByText("This plugin has no settings.")).toBeInTheDocument();
  });
});
