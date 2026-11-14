import { MantineProvider } from "@mantine/core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { IconPicker } from "./icon-picker";

describe("IconPicker", () => {
  beforeEach(() => localStorage.clear());

  it("narrows to a flat grid while searching and reports the chosen icon", async () => {
    const onChange = vi.fn();
    render(
      <MantineProvider>
        <IconPicker value="photo" onChange={onChange} />
      </MantineProvider>,
    );
    expect(screen.queryByRole("heading", { name: "Recent" })).toBeNull();
    expect(screen.getByRole("heading", { name: "General" })).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Search icons"), "plane");
    expect(screen.getByRole("heading", { name: /icons$/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Travel" }));
    expect(onChange).toHaveBeenCalledWith("plane");
    expect(JSON.parse(localStorage.getItem("lumen.cover.recent-icons")!)).toEqual(["plane"]);
    await userEvent.clear(screen.getByLabelText("Search icons"));
    await userEvent.type(screen.getByLabelText("Search icons"), "zzzz");
    expect(screen.getByText("No icon by that name.")).toBeInTheDocument();
  });
});
