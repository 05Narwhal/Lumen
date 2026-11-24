// The two values a build bakes in are public by nature (a minisign public key, and an application
// id every Discord client sees), so they are tracked files and a tag builds the same binary
// anywhere. Environment variables still override them (src-tauri/build.rs).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { MANUAL_STEPS } from "../oss/export-public.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (p) => readFileSync(join(root, p), "utf8");

function ignored(path) {
  try {
    execFileSync("git", ["check-ignore", "-q", path], { cwd: root, stdio: "ignore" });
    return true;
  } catch (e) {
    if (e.status === 1) return false;
    throw e;
  }
}

describe("tracked public values", () => {
  it("tracks the plugin public key: its last non-empty line is a minisign key or its base64", () => {
    const last = read("src-tauri/plugin-pubkey.txt").split(/\r?\n/).map((l) => l.trim()).filter(Boolean).pop();
    expect(last).toMatch(/^[A-Za-z0-9+/]{40,}={0,2}$/);
  });

  it("tracks the Discord application id: digits only", () => {
    expect(read("src-tauri/discord_client_id.txt").trim()).toMatch(/^\d{17,20}$/);
  });

  it("does not git-ignore either file", () => {
    expect(ignored("src-tauri/plugin-pubkey.txt")).toBe(false);
    expect(ignored("src-tauri/discord_client_id.txt")).toBe(false);
  });

  it("release.yml no longer passes either value as a secret", () => {
    const yml = read(".github/workflows/release.yml");
    expect(yml).not.toContain("LUMEN_PLUGIN_PUBKEY");
    expect(yml).not.toContain("LUMEN_DISCORD_CLIENT_ID");
  });

  it("the public snapshot's manual steps name only the two signing secrets", () => {
    const steps = MANUAL_STEPS("x");
    expect(steps).toContain("TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
    expect(steps).not.toContain("LUMEN_PLUGIN_PUBKEY");
    expect(steps).not.toContain("LUMEN_DISCORD_CLIENT_ID");
  });
});
