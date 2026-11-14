import { describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { invoke, isAppError } from "./ipc";

describe("ipc.invoke", () => {
  it("passes through results", async () => {
    mockCommand("get_app_info", () => ({ version: "0.1.0" }));
    await expect(invoke("get_app_info")).resolves.toEqual({ version: "0.1.0" });
    expect(invokeMock).toHaveBeenCalledWith("get_app_info", undefined);
  });

  it("normalises unknown failures into AppError", async () => {
    invokeMock.mockRejectedValueOnce("socket closed");
    const err = (await invoke("x").catch((e: unknown) => e)) as { code: string; detail: string | null };
    expect(isAppError(err)).toBe(true);
    expect(err.code).toBe("unknown");
    expect(err.detail).toBe("socket closed");
  });
});
