import { describe, expect, it } from "vitest";
import { ffmpegGate, parseFfmpegVersion, rawlerDependents, setupSizeGate, tagRefGate } from "./gates.mjs";
import { evaluatePreflight } from "./preflight.mjs";
import { BUNDLE, FFMPEG_OUT, good, SOURCES } from "./test-facts.mjs";

const failing = (facts, version = "26.1.0-alpha") => evaluatePreflight(facts, { version }).filter((g) => !g.ok).map((g) => g.id);

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
    expect(evaluatePreflight(good(), { version: "26.1.0-alpha" }).map((g) => g.id)).not.toContain("licences-offer");
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

describe("tagRefGate", () => {
  it("accepts main and release/26.3 and refuses a feature branch", () => {
    expect(tagRefGate({ ref: "refs/tags/v26.3.1-stable", branches: ["main"] }, "26.3.1-stable").ok).toBe(true);
    expect(tagRefGate({ ref: "refs/tags/v26.3.1-stable", branches: ["release/26.3"] }, "26.3.1-stable").ok).toBe(true);
    expect(tagRefGate({ ref: "refs/tags/v26.3.1-stable", branches: ["round-17-updater", "wip"] }, "26.3.1-stable").ok).toBe(false);
  });

  it("refuses a tag whose name is not v<version>, and no branch at all", () => {
    expect(tagRefGate({ ref: "v26.3.0-stable", branches: ["main"] }, "26.3.1-stable").ok).toBe(false);
    expect(tagRefGate({ ref: "v26.3.1-stable", branches: [] }, "26.3.1-stable").ok).toBe(false);
    expect(tagRefGate({}, "26.3.1-stable").ok).toBe(false);
  });
});
