import { describe, expect, it } from "vitest";
import { evaluateGates, ffmpegGate, parseFfmpegVersion, rawlerDependents, setupSizeGate } from "./gates.mjs";

const FFMPEG_OUT = `ffmpeg version 9.0.1-essentials_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers
built with gcc 16.1.0 (Rev2, Built by MSYS2 project)
configuration: --enable-gpl --enable-version3 --enable-static --disable-w32threads --disable-autodetect --enable-libx264 --enable-mediafoundation
libavutil      61.  1.101 / 61.  1.101`;

const SOURCES = {
  ffmpeg: {
    version: "9.0.1",
    sidecarSha256: "aa",
    options: ["gpl", "version3", "static"],
    components: [
      { name: "FFmpeg", flag: null, url: "https://example/ffmpeg-9.0.1.tar.xz", sha256: "f1", file: "ffmpeg-9.0.1.tar.xz" },
      { name: "x264", flag: "libx264", url: "https://example/x264.tar.bz2", sha256: "x1", file: "x264.tar.bz2" },
      { name: "Media Foundation", flag: "mediafoundation", os: true },
    ],
  },
  others: [
    { name: "ExifTool", url: "https://example/Image-ExifTool.tar.gz", sha256: "e1", file: "Image-ExifTool.tar.gz" },
  ],
};

const BUNDLE = { "ffmpeg-9.0.1.tar.xz": "f1", "x264.tar.bz2": "x1", "Image-ExifTool.tar.gz": "e1" };

/** Facts as they are when every gate holds; each test breaks one. */
const good = (over = {}) => ({
  branch: "main",
  clean: true,
  changelogVersions: ["26.1.0-alpha"],
  versionFields: "26.1.0-alpha",
  previousVersion: null,
  sidecars: { ffmpeg: true, ffprobe: true, exiftool: true },
  noticesFresh: true,
  checks: { ok: true, failed: [] },
  pubkey: "dW50cnVzdGVkIGNvbW1lbnQ6",
  privateKeyInEnv: true,
  testOverrides: [],
  libraryCheck: { ok: true, from: 30, to: 31 },
  setupSizes: { nsis: 85_000_000, full: 88_000_000, slim: 4_000_000 },
  rawler: { dependents: ["photos-raw"], resources: ["photos_raw.dll"] },
  ffmpeg: { versionOutput: FFMPEG_OUT, sidecarSha256: "aa" },
  sources: SOURCES,
  sourceBundle: BUNDLE,
  releasesRepo: true,
  gates: { jamendo: { resolved: true }, esri: { resolved: true } },
  ...over,
});

const failing = (facts, version = "26.1.0-alpha") => evaluateGates(facts, { version }).filter((g) => !g.ok).map((g) => g.id);

describe("evaluateGates", () => {
  it("passes when every fact is good", () => {
    expect(failing(good())).toEqual([]);
    expect(failing(good(), "26.1.0-stable".replace("stable", "alpha"))).toEqual([]);
  });

  it.each([
    ["clean", { clean: false }],
    ["main", { branch: "round-17-updater" }],
    ["changelog", { changelogVersions: ["26.0.9-alpha"] }],
    ["version-fields", { versionFields: "26.0.9-alpha" }],
    ["next-version", { previousVersion: "26.1.0-alpha" }],
    ["sidecars", { sidecars: { ffmpeg: true, ffprobe: false, exiftool: true } }],
    ["notices", { noticesFresh: false }],
    ["checks", { checks: { ok: false, failed: ["yarn test"] } }],
    ["signing", { pubkey: "" }],
    ["signing", { privateKeyInEnv: false }],
    ["no-test-overrides", { testOverrides: ["LUMEN_BAKED_APP_DATA_DIR"] }],
    ["library-check", { libraryCheck: { ok: false, why: "migration failed" } }],
    ["setup-sizes", { setupSizes: { nsis: 85_000_000, full: 92_000_000, slim: 4_000_000 } }],
    ["setup-sizes", { setupSizes: undefined }],
    ["releases-repo", { releasesRepo: false }],
  ])("refuses when %s does not hold", (id, over) => {
    expect(failing(good(over))).toEqual([id]);
  });

  it("refuses a version outside the scheme", () => {
    expect(failing(good({ versionFields: "26.1.0", changelogVersions: ["26.1.0"] }), "26.1.0")).toContain("version");
  });

  it("refuses while rawler is linked anywhere but photos-raw, which is today's tree", () => {
    expect(failing(good({ rawler: { dependents: ["photos-media"], resources: [] } }))).toEqual(["rawler"]);
    expect(failing(good({ rawler: { dependents: ["photos-raw", "photos-media"], resources: ["photos_raw.dll"] } }))).toEqual(["rawler"]);
  });

  it("refuses when cargo tree could not be read, rather than taking it for no rawler", () => {
    expect(failing(good({ rawler: { dependents: null, resources: ["photos_raw.dll"] } }))).toEqual(["rawler"]);
    expect(failing(good({ rawler: undefined }))).toEqual(["rawler"]);
  });

  it("refuses photos-raw that is not in the bundle", () => {
    expect(failing(good({ rawler: { dependents: ["photos-raw"], resources: [] } }))).toEqual(["rawler"]);
  });

  it("asks for Jamendo and Esri only for a stable build", () => {
    const open = { gates: { jamendo: { resolved: false }, esri: { resolved: false } } };
    expect(failing(good({ ...open, versionFields: "26.1.0-beta", changelogVersions: ["26.1.0-beta"] }), "26.1.0-beta")).toEqual([]);
    expect(failing(good({ ...open, versionFields: "26.1.0-stable", changelogVersions: ["26.1.0-stable"] }), "26.1.0-stable")).toEqual(["stable-only"]);
  });

  it("names why, never prints a secret", () => {
    const out = evaluateGates(good({ privateKeyInEnv: false }), { version: "26.1.0-alpha" });
    const signing = out.find((g) => g.id === "signing");
    expect(signing.why).toMatch(/TAURI_SIGNING_PRIVATE_KEY/);
    expect(JSON.stringify(out)).not.toContain("dW50cnVzdGVkIGNvbW1lbnQ6");
  });
});

describe("FFmpeg's gate", () => {
  it("reads the version and every configure flag", () => {
    const v = parseFfmpegVersion(FFMPEG_OUT);
    expect(v.version).toBe("9.0.1");
    expect(v.enabled).toEqual(["gpl", "version3", "static", "libx264", "mediafoundation"]);
  });

  it("holds when every library has its source in the bundle", () => {
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT, sidecarSha256: "aa" }, SOURCES.ffmpeg, BUNDLE)).toEqual({ ok: true, why: [] });
  });

  it("refuses a library with no source recorded", () => {
    const out = FFMPEG_OUT.replace("--enable-libx264", "--enable-libx264 --enable-libx265");
    const r = ffmpegGate({ versionOutput: out, sidecarSha256: "aa" }, SOURCES.ffmpeg, BUNDLE);
    expect(r.ok).toBe(false);
    expect(r.why.join(" ")).toMatch(/libx265/);
  });

  it("refuses a component listed without a URL or hash", () => {
    const sources = { ...SOURCES.ffmpeg, components: SOURCES.ffmpeg.components.map((c) => (c.flag === "libx264" ? { ...c, url: null, sha256: null } : c)) };
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT, sidecarSha256: "aa" }, sources, BUNDLE).ok).toBe(false);
  });

  it("refuses a different FFmpeg than the one recorded", () => {
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT, sidecarSha256: "bb" }, SOURCES.ffmpeg, BUNDLE).ok).toBe(false);
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT.replace("9.0.1", "9.0.2"), sidecarSha256: "aa" }, SOURCES.ffmpeg, BUNDLE).ok).toBe(false);
  });

  it("refuses a bundle file with the wrong hash or missing", () => {
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT, sidecarSha256: "aa" }, SOURCES.ffmpeg, { ...BUNDLE, "x264.tar.bz2": "zz" }).ok).toBe(false);
    expect(ffmpegGate({ versionOutput: FFMPEG_OUT, sidecarSha256: "aa" }, SOURCES.ffmpeg, { "ffmpeg-9.0.1.tar.xz": "f1" }).ok).toBe(false);
  });

  it("refuses when the sidecar is not there to ask", () => {
    expect(ffmpegGate({ versionOutput: null, sidecarSha256: null }, SOURCES.ffmpeg, BUNDLE).ok).toBe(false);
  });
});

describe("the other sources", () => {
  it("refuses an ExifTool source missing or changed", () => {
    const { "Image-ExifTool.tar.gz": _, ...without } = BUNDLE;
    expect(failing(good({ sourceBundle: without }))).toEqual(["source-bundle"]);
    expect(failing(good({ sourceBundle: { ...BUNDLE, "Image-ExifTool.tar.gz": "zz" } }))).toEqual(["source-bundle"]);
  });

  it("needs no photos-raw archive: its source is the tagged public repository", () => {
    expect(BUNDLE).not.toHaveProperty("photos-raw-source.tar.gz");
    expect(failing(good())).toEqual([]);
  });

  it("no longer carries a licences-offer gate", () => {
    expect(evaluateGates(good(), { version: "26.1.0-alpha" }).map((g) => g.id)).not.toContain("licences-offer");
  });

  it("refuses a source with no URL or hash recorded", () => {
    const sources = { ...SOURCES, others: [{ name: "Strawberry Perl", url: null, sha256: null, file: null }] };
    expect(failing(good({ sources }))).toEqual(["source-bundle"]);
  });
});

describe("rawlerDependents", () => {
  it("reads cargo tree's inverted list", () => {
    expect(rawlerDependents("rawler v0.8.1\nphotos-media v26.1.0-alpha (X:\\repo\\crates\\photos-media)\n")).toEqual(["photos-media"]);
    expect(rawlerDependents("error: package ID specification `rawler` did not match any packages\n")).toEqual([]);
  });

  it("answers null for output it can't read, such as a failed call", () => {
    expect(rawlerDependents("")).toBeNull();
    expect(rawlerDependents(undefined)).toBeNull();
    expect(rawlerDependents("error: manifest path `X:\\Documents\\Coding\\Projects\\Desktop-Apps\\Tauri\\Photos` does not exist\n")).toBeNull();
  });
});

describe("setupSizeGate", () => {
  const MB = 1024 * 1024;
  const nsis = 85 * MB;

  it("holds at the edges: full = installer + 6 MB, slim = 6 MB", () => {
    expect(setupSizeGate({ nsis, full: nsis + 6 * MB, slim: 6 * MB })).toEqual({ ok: true, why: [] });
  });

  it("refuses a full Setup one byte over the installer + 6 MB, and says by how much it is", () => {
    const r = setupSizeGate({ nsis, full: nsis + 6 * MB + 1, slim: 4 * MB });
    expect(r.ok).toBe(false);
    expect(r.why).toHaveLength(1);
    expect(r.why[0]).toMatch(/full Setup is 91\.00 MB.*85\.00 MB \+ 6 MB/);
  });

  it("refuses a slim Setup one byte over 6 MB", () => {
    const r = setupSizeGate({ nsis, full: nsis + 4 * MB, slim: 6 * MB + 1 });
    expect(r.why).toEqual([expect.stringMatching(/slim Setup is 6\.00 MB, over 6 MB/)]);
  });

  it("names both when both are over", () => {
    expect(setupSizeGate({ nsis, full: nsis + 9 * MB, slim: 9 * MB }).why).toHaveLength(2);
  });

  it.each([undefined, {}, { nsis: 0, full: 1, slim: 1 }, { nsis, full: NaN, slim: 1 }])("never passes sizes it could not read: %j", (sizes) => {
    expect(setupSizeGate(sizes).ok).toBe(false);
  });
});
