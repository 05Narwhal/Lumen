import { render, screen, waitFor } from "@testing-library/react";
import { MotionGlobalConfig } from "motion/react";
import { beforeAll, describe, expect, it } from "vitest";
import { Fold } from "./gui";

beforeAll(() => {
  MotionGlobalConfig.skipAnimations = true;
});

describe("Fold", () => {
  it("mounts its children when open and drops them when closed", async () => {
    const { rerender } = render(<Fold open={false}>inside</Fold>);
    expect(screen.queryByText("inside")).toBeNull();
    rerender(<Fold open>inside</Fold>);
    expect(screen.getByText("inside")).toBeInTheDocument();
    rerender(<Fold open={false}>inside</Fold>);
    await waitFor(() => expect(screen.queryByText("inside")).toBeNull());
  });
});
