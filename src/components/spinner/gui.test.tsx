import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Spinner } from "./gui";

describe("Spinner", () => {
  it("is announced as a status with a label", () => {
    render(<Spinner label="Preparing thumbnails" />);
    expect(screen.getByRole("status", { name: "Preparing thumbnails" })).toBeInTheDocument();
  });
});
