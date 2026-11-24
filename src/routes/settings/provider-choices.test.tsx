import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { ProviderChoices } from "./provider-choices";

const mount = () =>
  render(
    <Providers>
      <ProviderChoices plugins={[]} />
    </Providers>,
  );

describe("ProviderChoices", () => {
  it("shows a choice only for a kind that two plugins could answer", async () => {
    invokeMock.mockClear();
    mockCommand("plugin_provider_choices", () => [
      { kind: "file-picker", options: [{ pluginId: "a.one", name: "One" }, { pluginId: "b.two", name: "Two" }], chosen: "a.one" },
      { kind: "reveal", options: [{ pluginId: "a.one", name: "One" }], chosen: "a.one" },
    ]);
    mount();
    expect(await screen.findByRole("combobox", { name: "File dialogs" })).toHaveValue("One");
    expect(screen.queryByText("Show in folder")).not.toBeInTheDocument();
  });
});
