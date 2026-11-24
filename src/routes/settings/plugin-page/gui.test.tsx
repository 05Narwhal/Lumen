import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import { PluginPage } from "./gui";

const plugin: PluginInfo = {
  id: "jl.disk-atlas",
  name: "Disk Atlas",
  version: "1.0.0",
  author: "JL Studios",
  description: "",
  license: "GPL-3.0-or-later",
  trust: { type: "jlStudios" },
  kinds: [],
  permissions: [],
  enabled: true,
  dev: false,
  state: { type: "ready" },
  iconUrl: null,
  settings: [],
  contributions: [],
  sections: [
    {
      title: "Dialogs",
      description: null,
      controls: [
        { kind: "toggle", key: "use_picker", label: "Use Disk Atlas for file dialogs", help: null, value: true, choices: [], min: null, max: null, step: null, requires: null, app: "disk-atlas", action: null, body: null },
        { kind: "text", key: "note", label: "A note", help: null, value: "", choices: [], min: null, max: null, step: null, requires: null, app: null, action: null, body: null },
      ],
    },
  ],
  homepage: "https://example.com/disk-atlas", authorUrl: null, repository: null, screenshots: [], hasReadme: false,
  detectIds: ["disk-atlas"],
  assets: [],
};

const show = () =>
  render(
    <Providers>
      <MemoryRouter>
      <PluginPage plugin={plugin} onBack={() => {}} reload={() => {}} />
      </MemoryRouter>
    </Providers>,
  );

describe("PluginPage", () => {
  it("offers Locate and disables a setting that needs an app that isn't found", async () => {
    mockCommand("plugin_detect", () => [{ id: "disk-atlas", name: "Disk Atlas", path: null, version: null, located: false }]);
    show();
    expect(await screen.findByText("Disk Atlas not found")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Locate…" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use Disk Atlas for file dialogs" })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "A note" })).toBeEnabled();
    expect(screen.getByText("Dialogs")).toBeInTheDocument();
  });

  it("shows where the app is and enables the setting once found", async () => {
    mockCommand("plugin_detect", () => [{ id: "disk-atlas", name: "Disk Atlas", path: "C:/Apps/disk-atlas.exe", version: "0.4.2", located: false }]);
    show();
    expect(await screen.findByText("Disk Atlas found · 0.4.2")).toBeInTheDocument();
    expect(screen.getByText("C:/Apps/disk-atlas.exe")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use Disk Atlas for file dialogs" })).toBeEnabled();
  });
});
