import { describe, expect, it } from "vitest";
import { releasePlan } from "./channels.mjs";

const changelog = ["# Changelog", "", "## 26.4.0-alpha", "", "New things.", "", "## 26.3.5-stable · security", "", "Fixed a vulnerability.", "", "## 26.3.4-stable", "", "Old.", ""].join("\n");
const args = (version) => ({ version, changelog, installer: `Lumen_${version}_x64-setup.exe`, signature: " c2ln\n", now: "2026-11-15T00:00:00Z" });

describe("releasePlan", () => {
  it("marks a build whose changelog heading says `· security`", () => {
    const plan = releasePlan(args("26.3.5-stable"));
    expect(plan.security).toBe(true);
    expect(plan.notes).toBe("Fixed a vulnerability.");
  });

  it("leaves every other build unmarked", () => {
    expect(releasePlan(args("26.4.0-alpha")).security).toBe(false);
    expect(releasePlan(args("26.3.4-stable")).security).toBe(false);
  });

  it("carries the installer, the trimmed signature and the time", () => {
    const plan = releasePlan(args("26.4.0-alpha"));
    expect(plan).toMatchObject({ version: "26.4.0-alpha", files: [{ name: "Lumen_26.4.0-alpha_x64-setup.exe" }], signature: "c2ln", now: "2026-11-15T00:00:00Z" });
  });

  it("refuses a version outside the scheme", () => {
    expect(() => releasePlan(args("26.4.0"))).toThrow(/26\.X\.Y/);
  });
});
