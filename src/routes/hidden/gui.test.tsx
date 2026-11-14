import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useNavigate } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { HiddenRoute } from "./gui";

let vault = { hasPassword: false, count: 0, unlocked: false };

function Away() {
  const navigate = useNavigate();
  return (
    <button type="button" onClick={() => navigate("/library")}>
      leave
    </button>
  );
}

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/hidden"]}>
        <Routes>
          <Route
            path="/hidden"
            element={
              <>
                <HiddenRoute />
                <Away />
              </>
            }
          />
          <Route path="/library" element={<p>library</p>} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("the Hidden folder", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("vault_status", () => vault);
    mockCommand("vault_set_password", () => {
      vault = { ...vault, hasPassword: true, unlocked: true };
      return null;
    });
    mockCommand("vault_unlock", (a) => {
      const ok = a?.password === "open sesame";
      if (ok) vault = { ...vault, unlocked: true };
      return ok;
    });
    mockCommand("vault_reset_password", () => {
      vault = { ...vault, unlocked: true };
      return null;
    });
    mockCommand("vault_lock", () => {
      vault = { ...vault, unlocked: false };
      return null;
    });
  });
  afterEach(() => {
    for (const p of props) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
    vi.useRealTimers();
  });

  it("asks for a password the first time, twice, and then opens", async () => {
    vault = { hasPassword: false, count: 0, unlocked: false };
    mount();
    const [first, again] = await screen.findAllByLabelText(/password/i);
    await userEvent.type(first, "open sesame");
    await userEvent.type(again, "open sesaem");
    await userEvent.click(screen.getByRole("button", { name: "Set password" }));
    expect(screen.getByText("The passwords do not match.")).toBeInTheDocument();
    await userEvent.clear(again);
    await userEvent.type(again, "open sesame");
    await userEvent.click(screen.getByRole("button", { name: "Set password" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("vault_set_password", { password: "open sesame" }));
    expect(await screen.findByText(/Nothing hidden yet/)).toBeInTheDocument();
  });

  it("opens with the right password and not the wrong one, then shows only its photos", async () => {
    vault = { hasPassword: true, count: 3, unlocked: false };
    mount();
    const field = await screen.findByLabelText("Password");
    await userEvent.type(field, "wrong{Enter}");
    expect(await screen.findByText("That is not the password.")).toBeInTheDocument();
    await userEvent.clear(field);
    await userEvent.type(field, "open sesame{Enter}");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ vault: true }) })),
    );
  });

  it("locks when you leave it", async () => {
    vault = { hasPassword: true, count: 3, unlocked: true };
    mount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.anything()));
    await userEvent.click(screen.getByRole("button", { name: "leave" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("vault_lock", undefined));
  });

  it("resets a forgotten password only after a warning and a wait", async () => {
    vault = { hasPassword: true, count: 3, unlocked: false };
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Forgot password?" }));
    const go = await screen.findByRole("button", { name: /Choose a new password/ });
    expect(go).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    await waitFor(() => expect(go).toBeEnabled());
    await userEvent.click(go);
    const [first, again] = await screen.findAllByLabelText(/password/i);
    await userEvent.type(first, "fresh-start");
    await userEvent.type(again, "fresh-start");
    await userEvent.click(screen.getByRole("button", { name: "Set password" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("vault_reset_password", { password: "fresh-start" }));
  });
});
