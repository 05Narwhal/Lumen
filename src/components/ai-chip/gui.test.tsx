import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { AI_CHIP_LINGER_MS, AiChip, MCP_STATUS } from ".";

const settings = (enabled: boolean) => ({ appearance: { reduceMotion: false }, aiAssistants: { enabled, port: 47820, token: "t" } });
const status = (running: boolean) => emitMock(MCP_STATUS, { client: "claude-code", tool: "edit", running });

describe("AiChip", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockLibraryDefaults();
    mockCommand("get_settings", () => settings(true));
    mockCommand("update_settings", (a) => (a as { settings: unknown }).settings);
  });
  afterEach(() => vi.useRealTimers());

  it("shows the client while it calls, a spinner while a tool runs, and goes after a quiet minute", async () => {
    render(<AiChip />, { wrapper: Providers });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_settings", undefined));
    expect(screen.queryByRole("button")).toBeNull();
    await act(async () => status(true));
    expect(await screen.findByRole("button", { name: "claude-code is editing" })).toBeTruthy();
    expect(screen.getByRole("progressbar", { name: "Working" })).toBeTruthy();
    vi.useFakeTimers();
    await act(async () => status(false));
    expect(screen.queryByRole("progressbar")).toBeNull();
    await act(async () => vi.advanceTimersByTime(AI_CHIP_LINGER_MS + 10));
    expect(screen.queryByRole("button", { name: /is editing/ })).toBeNull();
  });

  it("turns AI assistants off from its popover", async () => {
    render(<AiChip />, { wrapper: Providers });
    await act(async () => status(true));
    fireEvent.click(await screen.findByRole("button", { name: "claude-code is editing" }));
    fireEvent.click(await screen.findByRole("button", { name: "Turn off" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("update_settings", { settings: settings(false) }));
  });
});
