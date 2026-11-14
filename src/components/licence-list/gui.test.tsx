import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { mockCommand } from "@/test/tauri-mock";
import { Providers } from "@/app";
import { LicenceList } from "./gui";

function mount() {
  render(
    <Providers>
      <LicenceList />
    </Providers>,
  );
}

describe("LicenceList", () => {
  it("licences_plugins_section: lists each plugin and its assets, and hides when none", async () => {
    mockCommand("list_plugins", () => [
      { id: "a.b", name: "Ember", version: "1.0.0", license: "MIT", assets: [{ path: "fonts/ember.woff2", license: "OFL-1.1" }] },
    ]);
    mount();
    const sec = await screen.findByRole("region", { name: "Plugins" });
    expect(sec).toHaveTextContent("Ember");
    expect(sec).toHaveTextContent("1.0.0");
    expect(sec).toHaveTextContent("MIT");
    expect(sec).toHaveTextContent("fonts/ember.woff2 · OFL-1.1");
  });

  it("hides the Plugins section when none are installed", async () => {
    mockCommand("list_plugins", () => []);
    mount();
    await screen.findByRole("region", { name: "Rust libraries" });
    expect(screen.queryByRole("region", { name: "Plugins" })).toBeNull();
  });

  it("shows every group with its count once the notices load", async () => {
    mount();
    expect(await screen.findByRole("region", { name: "Programs Lumen runs" })).toBeInTheDocument();
    for (const title of ["Machine-learning models", "Maps, places and music", "Notices", "Rust libraries", "JavaScript libraries and fonts"])
      expect(screen.getByRole("region", { name: title })).toBeInTheDocument();
    expect(screen.getByText(/FFmpeg build includes x264 and x265/)).toBeInTheDocument();
    expect(screen.getByText(/Map data © OpenStreetMap contributors/)).toBeInTheDocument();
    expect(screen.getByText(/based in part on the work of the Independent JPEG Group/)).toBeInTheDocument();
  });

  it("links to the public source code instead of carrying a written offer", async () => {
    mount();
    const link = await screen.findByRole("button", { name: /Source code/ });
    await userEvent.click(link);
    expect(openUrl).toHaveBeenCalledWith("https://github.com/05Narwhal/lumen");
    expect(screen.queryByText(/Written offer/)).toBeNull();
    expect(screen.queryByText(/three years/)).toBeNull();
  });

  it("finds a library by name and opens its licence text", async () => {
    mount();
    await screen.findByRole("region", { name: "Rust libraries" });
    await userEvent.type(screen.getByRole("textbox", { name: "Find a licence" }), "rawler");
    await waitFor(() => expect(screen.queryByRole("region", { name: "Programs Lumen runs" })).toBeNull());
    const rust = screen.getByRole("region", { name: "Rust libraries" });
    const entry = within(rust).getByRole("button", { name: /rawler/ });
    expect(entry).toHaveTextContent("LGPL-2.1");
    await userEvent.click(entry);
    expect(await within(rust).findByText(/GNU LESSER GENERAL PUBLIC LICENSE/)).toBeInTheDocument();
  });

  it("says so when nothing matches", async () => {
    mount();
    await screen.findByRole("region", { name: "Rust libraries" });
    await userEvent.type(screen.getByRole("textbox", { name: "Find a licence" }), "zzzz-nothing");
    expect(await screen.findByText(/Nothing matches/)).toBeInTheDocument();
  });
});
