import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Toaster } from "./gui";
import { toast, toastStore } from "./store";

describe("Toaster", () => {
  it("renders a toast and runs undo", async () => {
    toastStore.set({ toasts: [] });
    const undo = vi.fn();
    render(<Toaster />);
    toast({ message: "Removed from album", undo });
    expect(await screen.findByText("Removed from album")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(undo).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByText("Removed from album")).toBeNull());
  });
});
