import { describe, expect, it } from "vitest";
import { TEST_OVERRIDES } from "./gates.mjs";
import { branchesFrom, bundleFromAssets, checksFromRuns, evaluatePreflight, formatReport, judgeLibraryChecks, migrationsChangedFrom, parseArgs, PHASES, previousVersionFrom, resourceNames, runGates } from "./preflight.mjs";
import { BUNDLE, good } from "./test-facts.mjs";

const failing = (facts, version = "26.1.0-alpha") => evaluatePreflight(facts, { version }).filter((g) => !g.ok).map((g) => g.id);

describe("no-migration (a silent security install never rewrites a schema on a stable line)", () => {
  const stableFix = (over = {}) => good({ ref: "refs/tags/v26.3.5-stable", versionFields: "26.3.5-stable", changelogVersions: ["26.3.5-stable"], previousVersion: "26.3.4-stable", branches: ["release/26.3"], security: true, ...over });

  it("refuses a security tag on release/* whose migrations differ from the line's previous tag", () => {
    expect(failing(stableFix({ migrationsChanged: true }), "26.3.5-stable")).toEqual(["no-migration"]);
  });

  it("passes a security tag with the same migrations", () => {
    expect(failing(stableFix(), "26.3.5-stable")).toEqual([]);
  });

  it("refuses when it could not be compared, rather than let a schema change through", () => {
    expect(failing(stableFix({ migrationsChanged: null }), "26.3.5-stable")).toEqual(["no-migration"]);
  });

  it("holds a stable security tag cut from main to the same rule", () => {
    expect(failing(stableFix({ branches: ["main"], migrationsChanged: true }), "26.3.5-stable")).toEqual(["no-migration"]);
  });

  it("lets a preview, which may migrate, pass", () => {
    const alpha = good({ ref: "refs/tags/v26.4.0-alpha", versionFields: "26.4.0-alpha", changelogVersions: ["26.4.0-alpha"], security: true, migrationsChanged: true });
    expect(failing(alpha, "26.4.0-alpha")).toEqual([]);
  });

  it("lets a build that is not a security fix migrate", () => {
    expect(failing(stableFix({ security: false, migrationsChanged: true }), "26.3.5-stable")).toEqual([]);
  });

  it("migrationsChangedFrom reads git diff: empty means unchanged, a failure means unknown", () => {
    expect(migrationsChangedFrom({ ok: true, out: "" })).toBe(false);
    expect(migrationsChangedFrom({ ok: true, out: "src-tauri/crates/photos-core/migrations/0040_x.sql\n" })).toBe(true);
    expect(migrationsChangedFrom({ ok: false, out: "fatal: bad revision" })).toBeNull();
  });
});

describe("evaluatePreflight", () => {
  it("passes when every fact is good", () => {
    expect(failing(good())).toEqual([]);
  });

  it.each([
    ["tag-ref", { branches: ["round-17-updater"] }],
    ["tag-ref", { ref: "refs/tags/v26.1.1-alpha" }],
    ["changelog", { changelogVersions: ["26.0.9-alpha"] }],
    ["version-fields", { versionFields: "26.0.9-alpha" }],
    ["next-version", { previousVersion: "26.1.0-alpha" }],
    ["notices", { noticesFresh: false }],
    ["checks", { checks: { ok: false, failed: ["frontend"] } }],
    ["env", { testOverrides: ["LUMEN_BAKED_APP_DATA_DIR"] }],
    ["library-check", { libraryCheck: { ok: false, why: "migration failed" } }],
    ["setup-sizes", { setupSizes: { nsis: 85_000_000, full: 92_000_000, slim: 4_000_000 } }],
    ["setup-sizes", { setupSizes: undefined }],
    ["rawler", { rawler: { dependents: ["photos-media"], resources: [] } }],
    ["rawler", { rawler: { dependents: null, resources: ["photos_raw.dll"] } }],
    ["rawler", { rawler: { dependents: ["photos-raw"], resources: [] } }],
    ["ffmpeg-source", { ffmpeg: { versionOutput: null, sidecarSha256: null } }],
    ["source-bundle", { sourceBundle: { ...BUNDLE, "Image-ExifTool.tar.gz": "zz" } }],
  ])("refuses when %s does not hold", (id, over) => {
    expect(failing(good(over))).toEqual([id]);
  });

  it("refuses a version outside the scheme", () => {
    expect(failing(good({ versionFields: "26.1.0", changelogVersions: ["26.1.0"], ref: "v26.1.0" }), "26.1.0")).toContain("version");
  });

  it("env refuses LUMEN_UPDATE_TEST_BASE and every other test override", () => {
    for (const name of TEST_OVERRIDES) expect(failing(good({ testOverrides: [name] }))).toEqual(["env"]);
    expect(TEST_OVERRIDES).toContain("LUMEN_UPDATE_TEST_BASE");
  });

  it("holds a -stable tag to the same gates as any other: there is no stable-only gate", () => {
    const stable = good({ ref: "v26.1.0-stable", versionFields: "26.1.0-stable", changelogVersions: ["26.1.0-stable"] });
    const out = evaluatePreflight(stable, { version: "26.1.0-stable" });
    expect(out.map((g) => g.id)).not.toContain("stable-only");
    expect(out.filter((g) => !g.ok)).toEqual([]);
  });

  it("has none of the dropped local gates", () => {
    const ids = evaluatePreflight(good(), { version: "26.1.0-alpha" }).map((g) => g.id);
    for (const dropped of ["clean", "main", "signing", "releases-repo", "sidecars", "no-test-overrides"]) expect(ids).not.toContain(dropped);
  });

  it("splits into what runs before the build and what needs the built candidate", () => {
    expect(PHASES.after).toEqual(["library-check", "setup-sizes"]);
    const ids = evaluatePreflight(good(), { version: "26.1.0-alpha" }).map((g) => g.id);
    expect([...PHASES.before, ...PHASES.after].sort()).toEqual([...ids].sort());
  });
});

describe("checksFromRuns", () => {
  const run = (name, conclusion, status = "completed") => ({ name, conclusion, status });
  const ci = [run("frontend", "success"), run("scripts", "success"), run("rust (windows-latest)", "success")];

  it("holds when every required CI job succeeded on the commit", () => {
    expect(checksFromRuns(ci)).toEqual({ ok: true, failed: [] });
  });

  it("names a job that failed, is missing or is still running", () => {
    expect(checksFromRuns([run("frontend", "failure"), ...ci.slice(1)]).failed).toEqual(["frontend"]);
    expect(checksFromRuns(ci.slice(0, 2)).failed).toEqual(["rust (windows-latest)"]);
    expect(checksFromRuns([run("frontend", null, "in_progress"), ...ci.slice(1)]).failed).toEqual(["frontend"]);
  });

  it("takes an earlier green run when the tag push started a second one that is still running", () => {
    expect(checksFromRuns([...ci, run("frontend", null, "in_progress")]).ok).toBe(true);
  });
});

describe("judgeLibraryChecks", () => {
  const ok = (from, to = 39) => `{"ok":true,"from":${from},"to":${to},"version":"26.3.0-alpha"}`;
  const answers = [
    { name: "26.1.0-alpha-schema-37", stdout: `noise
${ok(37)}
` },
    { name: "26.2.0-alpha-schema-39", stdout: ok(39) },
  ];

  it("holds when every fixture opened from its own schema", () => {
    expect(judgeLibraryChecks(answers)).toEqual({ ok: true, from: 37, to: 39 });
  });

  it("names the fixture that did not open and why", () => {
    const r = judgeLibraryChecks([answers[0], { name: "26.2.0-alpha-schema-39", stdout: '{"ok":false,"why":"v40: no such column"}' }]);
    expect(r.ok).toBe(false);
    expect(r.why).toMatch(/26\.2\.0-alpha-schema-39.*v40: no such column/);
  });

  it("refuses an answer it cannot read, a fixture opened from the wrong schema, and no fixtures at all", () => {
    expect(judgeLibraryChecks([{ name: "x-schema-37", stdout: "" }]).ok).toBe(false);
    expect(judgeLibraryChecks([{ name: "x-schema-37", stdout: ok(36) }]).ok).toBe(false);
    expect(judgeLibraryChecks([]).ok).toBe(false);
  });
});

describe("the CLI's pure parts", () => {
  it("parseArgs reads the phase, the version and the after-build files", () => {
    expect(parseArgs(["before", "26.2.1-alpha"])).toEqual({ phase: "before", version: "26.2.1-alpha" });
    expect(parseArgs(["after", "26.2.1-alpha", "--sizes", "s.json", "--exe", "l.exe"])).toEqual({ phase: "after", version: "26.2.1-alpha", sizes: "s.json", exe: "l.exe" });
  });

  it("parseArgs refuses a missing phase or version, and an after run without its files", () => {
    expect(() => parseArgs([])).toThrow(/usage/);
    expect(() => parseArgs(["before"])).toThrow(/usage/);
    expect(() => parseArgs(["during", "26.2.1-alpha"])).toThrow(/usage/);
    expect(() => parseArgs(["after", "26.2.1-alpha"])).toThrow(/--sizes/);
  });

  it("branchesFrom drops the remote prefix, the HEAD pointer and blank lines", () => {
    const out = ["  origin/HEAD -> origin/main", "  origin/main", "  origin/release/26.3", "", ""].join("\n");
    expect(branchesFrom(out)).toEqual(["main", "release/26.3"]);
  });

  it("previousVersionFrom is the highest published version except the one being released", () => {
    const rel = (tagName, isDraft = false) => ({ tagName, isDraft });
    const list = [rel("v26.1.0-alpha"), rel("v26.2.0-alpha"), rel("channels"), rel("ffmpeg-sources-n9.0.2-47"), rel("v26.2.1-alpha", true), rel("v26.2.2-alpha")];
    expect(previousVersionFrom(list, "v26.2.2-alpha")).toBe("26.2.0-alpha");
    expect(previousVersionFrom([rel("channels")], "v26.1.0-alpha")).toBeNull();
  });

  it("previousVersionFrom looks within the tagged build's line, so a stable hotfix can follow a newer alpha", () => {
    const rel = (tagName, isDraft = false) => ({ tagName, isDraft });
    const list = [rel("v26.3.4-stable"), rel("v26.4.2-alpha"), rel("channels")];
    expect(previousVersionFrom(list, "v26.3.5-stable")).toBe("26.3.4-stable");
    expect(previousVersionFrom([rel("v26.3.5-stable")], "v26.4.0-alpha")).toBe("26.3.5-stable");
  });

  it("a stable hotfix after a newer alpha passes next-version, and a repeated number does not", () => {
    const hotfix = { ref: "refs/tags/v26.3.5-stable", versionFields: "26.3.5-stable", changelogVersions: ["26.3.5-stable"], branches: ["release/26.3"] };
    expect(failing(good({ ...hotfix, previousVersion: "26.3.4-stable" }), "26.3.5-stable")).toEqual([]);
    expect(failing(good({ ...hotfix, previousVersion: "26.3.5-stable" }), "26.3.5-stable")).toEqual(["next-version"]);
  });

  it("bundleFromAssets maps a file to its SHA-256 and leaves out an asset with no digest", () => {
    const assets = [{ name: "a.tar.gz", digest: "sha256:ab12" }, { name: "b.tar.gz" }];
    expect(bundleFromAssets(assets)).toEqual({ "a.tar.gz": "ab12" });
  });

  it("resourceNames expands the one folder build.rs fills with the replaceable library", () => {
    const list = (dir) => (dir === "binaries/raw" ? ["photos_raw.dll"] : []);
    expect(resourceNames(["binaries/raw/*", "binaries/exiftool_files/"], list)).toContain("photos_raw.dll");
    expect(resourceNames(["binaries/other/*"], () => [])).toEqual(["*"]);
  });

  it("resourceNames before the first build still sees that the bundle takes binaries/raw", () => {
    expect(resourceNames(["binaries/raw/*"], () => []).some((n) => /photos_raw/.test(n))).toBe(true);
  });

  it("runGates keeps only the gates of its phase, in the order of the full list", () => {
    const before = runGates(good(), { version: "26.1.0-alpha", phase: "before" }).map((g) => g.id);
    expect(before).not.toContain("library-check");
    expect(before).toContain("tag-ref");
    expect(runGates(good(), { version: "26.1.0-alpha", phase: "after" }).map((g) => g.id)).toEqual(["library-check", "setup-sizes"]);
  });

  it("formatReport prints one line per gate and says why a refused one failed", () => {
    const text = formatReport([{ id: "changelog", ok: true, why: "" }, { id: "notices", ok: false, why: "stale" }]);
    expect(text).toMatch(/ok\s+changelog/);
    expect(text).toMatch(/FAIL\s+notices.*stale/);
  });
});
