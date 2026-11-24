import { describe, expect, it } from "vitest";
import { channelsUpdatedBy, compareVersions, isNextVersion, isSecurityRelease, notesFor, previousInLine, parseVersion, writeSetupVersionFields, writeVersionFields } from "./lib.mjs";

describe("parseVersion", () => {
  it("reads the scheme", () => {
    expect(parseVersion("26.1.0-alpha")).toEqual({ x: 1, y: 0, channel: "alpha", numeric: "26.1.0" });
    expect(parseVersion("26.12.3-stable").channel).toBe("stable");
  });

  it.each(["26.1.0", "2026.1.0", "26.1-alpha", "26.1.0-rc", "27.1.0-alpha", "v26.1.0-alpha", "26.01.0-alpha", " 26.1.0-alpha"])("refuses %s", (v) => {
    expect(() => parseVersion(v)).toThrow();
  });
});

describe("channelsUpdatedBy", () => {
  it("a build reaches its own channel and every less stable one", () => {
    expect(channelsUpdatedBy("stable")).toEqual(["stable", "beta", "alpha"]);
    expect(channelsUpdatedBy("beta")).toEqual(["beta", "alpha"]);
    expect(channelsUpdatedBy("alpha")).toEqual(["alpha"]);
  });
});

describe("compareVersions", () => {
  it("orders by number, then channel", () => {
    expect(compareVersions("26.1.0-alpha", "26.1.1-alpha")).toBeLessThan(0);
    expect(compareVersions("26.2.0-alpha", "26.1.9-stable")).toBeGreaterThan(0);
    expect(compareVersions("26.1.0-alpha", "26.1.0-beta")).toBeLessThan(0);
    expect(compareVersions("26.1.0-stable", "26.1.0-stable")).toBe(0);
  });
});

describe("isNextVersion", () => {
  it("a new build needs a new number, so one number means one build", () => {
    expect(isNextVersion("26.1.0-alpha", "26.1.1-alpha")).toBe(true);
    expect(isNextVersion("26.1.3-alpha", "26.1.4-beta")).toBe(true);
    expect(isNextVersion("26.1.3-alpha", "26.2.0-alpha")).toBe(true);
    expect(isNextVersion("26.1.3-alpha", "26.1.3-beta")).toBe(false);
    expect(isNextVersion("26.1.3-alpha", "26.1.3-alpha")).toBe(false);
    expect(isNextVersion("26.1.3-alpha", "26.1.2-alpha")).toBe(false);
  });

  it("Y resets when X goes up", () => {
    expect(isNextVersion("26.1.3-alpha", "26.2.1-alpha")).toBe(false);
  });

  it("the first release has nothing before it", () => {
    expect(isNextVersion(null, "26.1.0-alpha")).toBe(true);
  });
});

describe("writeVersionFields", () => {
  const before = {
    packageJson: `{\n  "name": "lumen",\n  "version": "2026.1.0",\n  "type": "module"\n}\n`,
    tauriConf: `{\n  "version": "2026.1.0",\n  "bundle": {\n    "windows": {\n      "wix": {\n        "version": "26.1.0"\n      }\n    }\n  }\n}\n`,
    cargoToml: `[workspace.package]\nversion = "2026.1.0"\nedition = "2021"\n\n[package]\nname = "lumen"\nversion.workspace = true\n\n[dependencies]\nrayon = { version = "1" }\n`,
  };

  it("changes the version and nothing else", () => {
    const after = writeVersionFields(before, "26.3.2-beta");
    expect(JSON.parse(after.packageJson).version).toBe("26.3.2-beta");
    expect(after.packageJson).toContain(`"name": "lumen"`);
    const conf = JSON.parse(after.tauriConf);
    expect(conf.version).toBe("26.3.2-beta");
    expect(conf.bundle.windows.wix.version).toBe("26.3.2");
    expect(after.cargoToml).toContain(`version = "26.3.2-beta"\nedition`);
    expect(after.cargoToml).toContain(`rayon = { version = "1" }`);
    expect(after.cargoToml).toContain(`version.workspace = true`);
  });

  it("refuses a version outside the scheme", () => {
    expect(() => writeVersionFields(before, "26.3.2")).toThrow();
  });
});

describe("writeSetupVersionFields", () => {
  const before = {
    packageJson: `{
  "name": "lumen-setup",
  "version": "26.1.0-alpha"
}
`,
    tauriConf: `{
  "productName": "Lumen Setup",
  "version": "26.1.0-alpha"
}
`,
    cargoToml: `[package]
name = "lumen-setup"
version = "26.1.0-alpha"
edition = "2021"

[dependencies]
tauri = { version = "2" }
`,
  };

  it("changes the three version fields and nothing else", () => {
    const after = writeSetupVersionFields(before, "26.2.0-beta");
    expect(JSON.parse(after.packageJson)).toEqual({ name: "lumen-setup", version: "26.2.0-beta" });
    expect(JSON.parse(after.tauriConf).version).toBe("26.2.0-beta");
    expect(after.cargoToml).toContain(`[package]
name = "lumen-setup"
version = "26.2.0-beta"
edition`);
    expect(after.cargoToml).toContain(`tauri = { version = "2" }`);
  });

  it("refuses a file with no version field", () => {
    expect(() => writeSetupVersionFields({ ...before, cargoToml: "[package]\nname = \"x\"\n" }, "26.2.0-beta")).toThrow(/Cargo.toml/);
  });
});

describe("notesFor", () => {
  const log = "# Changelog\n\n## 26.1.1-alpha\n\n- Faster grid.\n- Fixes.\n\n## 26.1.0-alpha\n\n- First alpha.\n";

  it("takes the section for a version and nothing after it", () => {
    expect(notesFor(log, "26.1.1-alpha")).toBe("- Faster grid.\n- Fixes.");
    expect(notesFor(log, "26.1.0-alpha")).toBe("- First alpha.");
  });

  it("is empty for a version with no entry", () => {
    expect(notesFor(log, "26.2.0-alpha")).toBe("");
  });
});

describe("the version rule is per line (26.X)", () => {
  it("a stable hotfix follows the last build of its own line, not the newest build overall", () => {
    const published = ["26.3.0-stable", "26.3.4-stable", "26.4.2-alpha"];
    const prev = previousInLine(published, "26.3.5-stable");
    expect(prev).toBe("26.3.4-stable");
    expect(isNextVersion(prev, "26.3.5-stable")).toBe(true);
  });

  it("a hotfix that does not go up within its line is still refused", () => {
    const published = ["26.3.4-stable", "26.4.2-alpha"];
    expect(isNextVersion(previousInLine(published, "26.3.4-beta"), "26.3.4-beta")).toBe(false);
    expect(isNextVersion(previousInLine(published, "26.3.3-stable"), "26.3.3-stable")).toBe(false);
  });

  it("the first alpha of a new line follows the newest build of the lines before it", () => {
    const published = ["26.3.4-stable", "26.3.5-stable"];
    const prev = previousInLine(published, "26.4.0-alpha");
    expect(prev).toBe("26.3.5-stable");
    expect(isNextVersion(prev, "26.4.0-alpha")).toBe(true);
    expect(isNextVersion(prev, "26.4.1-alpha")).toBe(false);
  });

  it("a line cannot be started behind a newer one", () => {
    const prev = previousInLine(["26.4.2-alpha"], "26.3.0-stable");
    expect(isNextVersion(prev, "26.3.0-stable")).toBe(false);
  });

  it("has nothing before the very first release", () => {
    expect(previousInLine([], "26.1.0-alpha")).toBeNull();
  });
});

describe("isSecurityRelease", () => {
  const log = ["# Changelog", "", "## 26.3.5-stable · security", "", "Fixed a thing.", "", "## 26.4.0-alpha", "", "New.", "", "## 26.3.4-stable", ""].join("\n");
  it("is the heading's `· security` marker, for that version only", () => {
    expect(isSecurityRelease(log, "26.3.5-stable")).toBe(true);
    expect(isSecurityRelease(log, "26.4.0-alpha")).toBe(false);
    expect(isSecurityRelease(log, "26.3.4-stable")).toBe(false);
  });

  it("does not match a longer number that starts the same, or a mention in the notes", () => {
    const other = ["## 26.3.50-stable · security", "", "## 26.3.5-stable", "", "Not a · security release.", ""].join("\n");
    expect(isSecurityRelease(other, "26.3.5-stable")).toBe(false);
  });

  it("is false for a changelog with no such heading", () => {
    expect(isSecurityRelease("## 26.1.0-alpha\n","26.1.0-alpha")).toBe(false);
  });
});
