import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Segmented } from "./gui";

const data = [
  { label: "S", value: "s" as const },
  { label: "M", value: "m" as const },
  { label: "L", value: "l" as const },
];

function Harness({ onChange }: { onChange?: (v: "s" | "m" | "l") => void }) {
  const [v, setV] = useState<"s" | "m" | "l">("m");
  return (
    <Segmented
      label="Tile size"
      value={v}
      data={data}
      onChange={(next) => {
        setV(next);
        onChange?.(next);
      }}
    />
  );
}

describe("Segmented", () => {
  it("exposes a radio group and reports the chosen value", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    const group = screen.getByRole("radiogroup", { name: "Tile size" });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "M" })).toBeChecked();

    await userEvent.click(screen.getByRole("radio", { name: "L" }));
    expect(onChange).toHaveBeenCalledWith("l");
    expect(screen.getByRole("radio", { name: "L" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "M" })).not.toBeChecked();
  });

  it("renders the travelling pill only inside the selected option", async () => {
    render(<Harness />);
    const selected = screen.getByRole("radio", { name: "M" });
    expect(selected.querySelector(".segmented__pill")).not.toBeNull();
    expect(screen.getByRole("radio", { name: "S" }).querySelector(".segmented__pill")).toBeNull();

    await userEvent.click(screen.getByRole("radio", { name: "S" }));
    expect(screen.getByRole("radio", { name: "S" }).querySelector(".segmented__pill")).not.toBeNull();
    expect(document.querySelectorAll(".segmented__pill")).toHaveLength(1);
  });
});
