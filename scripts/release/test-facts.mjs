// Facts shared by the gate tests: a tiny FFmpeg, its source list and a bundle that matches.
export const FFMPEG_OUT = `ffmpeg version 9.0.1-essentials_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers
built with gcc 16.1.0 (Rev2, Built by MSYS2 project)
configuration: --enable-gpl --enable-version3 --enable-static --disable-w32threads --disable-autodetect --enable-libx264 --enable-mediafoundation
libavutil      61.  1.101 / 61.  1.101`;

export const SOURCES = {
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

export const BUNDLE = { "ffmpeg-9.0.1.tar.xz": "f1", "x264.tar.bz2": "x1", "Image-ExifTool.tar.gz": "e1" };


/** Facts as they are when every preflight gate holds; each test breaks one. */
export const good = (over = {}) => ({
  ref: "refs/tags/v26.1.0-alpha",
  branches: ["main"],
  changelogVersions: ["26.1.0-alpha"],
  versionFields: "26.1.0-alpha",
  previousVersion: null,
  security: false,
  migrationsChanged: false,
  noticesFresh: true,
  checks: { ok: true, failed: [] },
  testOverrides: [],
  libraryCheck: { ok: true, from: 30, to: 31 },
  setupSizes: { nsis: 85_000_000, full: 88_000_000, slim: 4_000_000 },
  rawler: { dependents: ["photos-raw"], resources: ["photos_raw.dll"] },
  ffmpeg: { versionOutput: FFMPEG_OUT, sidecarSha256: "aa" },
  sources: SOURCES,
  sourceBundle: BUNDLE,
  ...over,
});
