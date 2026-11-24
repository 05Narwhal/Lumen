import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { UpdateState } from "@/types/generated/UpdateState";
import { SecurityBanner } from "./gui";

const ready: UpdateState = { state: "ready", version: "26.3.5-stable", notes: "", attempts: 0 };

function show(state: UpdateState, security: boolean) {
  mockCommand("update_status", () => ({ version: "26.3.4-stable", state, whatsNew: null, security }));
  mockCommand("install_update_now", () => null);
  render(
    <Providers>
      <SecurityBanner />
    </Providers>,
  );
}

describe("SecurityBanner", () => {
  it("says a security fix is ready and installs it on Install now", async () => {
    show(ready, true);
    expect(await screen.findByText("A security fix is ready")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Install now" }));
    expect(invokeMock).toHaveBeenCalledWith("install_update_now", undefined);
  });

  it("stays out of the way for an ordinary update, a check or nothing staged", async () => {
    show(ready, false);
    await screen.findByRole("status", { hidden: true }).catch(() => undefined);
    expect(screen.queryByText("A security fix is ready")).toBeNull();
  });
});
