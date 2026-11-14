import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { EMPTY_QUERY } from "@/hooks/use-search";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import { MarkFilter } from "./gui";

function mount(query: AssetQuery = EMPTY_QUERY) {
  const onChange = vi.fn();
  render(<MarkFilter query={query} onChange={onChange} />);
  return onChange;
}

describe("MarkFilter", () => {
  it("shows only the picks, and a second click shows everything again", () => {
    let onChange = mount();
    fireEvent.click(screen.getByRole("button", { name: "Show picked" }));
    expect(onChange).toHaveBeenCalledWith({ flag: 1 });
    document.body.innerHTML = "";
    onChange = mount({ ...EMPTY_QUERY, flag: 1 });
    expect(screen.getByRole("button", { name: "Show picked" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Show picked" }));
    expect(onChange).toHaveBeenCalledWith({ flag: null });
  });

  it("adds and removes colour labels", () => {
    const onChange = mount({ ...EMPTY_QUERY, colorLabels: [1] });
    fireEvent.click(screen.getByRole("button", { name: "Show green label" }));
    expect(onChange).toHaveBeenCalledWith({ colorLabels: [1, 3] });
    fireEvent.click(screen.getByRole("button", { name: "Show red label" }));
    expect(onChange).toHaveBeenCalledWith({ colorLabels: [] });
  });
});
