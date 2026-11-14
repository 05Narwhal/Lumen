import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CoverStack, photoItems } from "./index";

describe("CoverStack", () => {
  // The protocol serves thumbnails at 256 and 1024 only; 512 answered 404 for every print.
  it("draws up to three photos from the 1024 thumbnails", () => {
    const { container } = render(<CoverStack items={photoItems([1, 2, 3, 4])} />);
    const imgs = [...container.querySelectorAll("img")];
    expect(imgs).toHaveLength(3);
    for (const img of imgs) expect(img.getAttribute("src")).toContain("s=1024");
    expect(container.querySelector(".cover-stack")?.getAttribute("data-count")).toBe("3");
  });

  it("draws an icon cover in a print without a photo", () => {
    const { container } = render(
      <CoverStack items={[{ kind: "photo", id: 5 }, { kind: "icon", style: { kind: "icon", shape: "box", icon: "star", color: "swatch-4" } }]} />,
    );
    const prints = container.querySelectorAll(".cover-stack__print");
    expect(prints).toHaveLength(2);
    expect(prints[0].querySelector("img")).not.toBeNull();
    expect(prints[1].querySelector("img")).toBeNull();
    expect(prints[1].querySelector("svg")).not.toBeNull();
  });
});
