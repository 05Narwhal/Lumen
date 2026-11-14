import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { archiveUrl, componentsFor, loadStages, parseScript, stagesFor } from "./btbn-sources.mjs";

const AOM = `#!/bin/bash

SCRIPT_REPO="https://aomedia.googlesource.com/aom"
SCRIPT_COMMIT="9ace39e02a34992b83fc7f7f2bfbbd2e79348c74"

ffbuild_depends() {
    echo base
    echo vmaf
}

ffbuild_configure() {
    echo --enable-libaom
}

ffbuild_unconfigure() {
    echo --disable-libaom
}
`;
const VMAF = `SCRIPT_REPO="https://github.com/Netflix/vmaf.git"
SCRIPT_COMMIT="4991d2b5aeb26391fbb85b63e3e86e7ad7a94b6e"

ffbuild_configure() {
    echo --enable-libvmaf
}
`;
const LAME = `SCRIPT_REPO="https://svn.code.sf.net/p/lame/svn/trunk/lame"
SCRIPT_REV="7113"

ffbuild_configure() {
    echo --enable-libmp3lame
}
`;
const NVC = `SCRIPT_REPO="https://github.com/FFmpeg/nv-codec-headers.git"
SCRIPT_COMMIT="f2053071ea93137255b6dcf42e9b4c9e0965a44e"

SCRIPT_REPO2="https://github.com/FFmpeg/nv-codec-headers.git"
SCRIPT_COMMIT2="ced4f8eba3ba5dd431932cba17928f0dffdaeb2b"

ffbuild_configure() {
    echo --enable-ffnvcodec --enable-cuda-llvm
}
`;
const UNUSED = `SCRIPT_REPO="https://github.com/example/unused.git"
SCRIPT_COMMIT="0000000000000000000000000000000000000001"

ffbuild_configure() {
    echo --enable-libunused
}
`;

let dir;
afterEach(() => dir && rmSync(dir, { recursive: true, force: true }));
function scriptsDir() {
  dir = mkdtempSync(join(tmpdir(), "lumen-scripts-"));
  const d = join(dir, "scripts.d");
  mkdirSync(join(d, "47-vulkan"), { recursive: true });
  writeFileSync(join(d, "50-aom.sh"), AOM);
  writeFileSync(join(d, "45-vmaf.sh"), VMAF);
  writeFileSync(join(d, "50-libmp3lame.sh"), LAME);
  writeFileSync(join(d, "50-ffnvcodec.sh"), NVC);
  writeFileSync(join(d, "50-unused.sh"), UNUSED);
  writeFileSync(join(d, "47-vulkan", "40-vulkan-headers.sh"), `SCRIPT_REPO="https://github.com/KhronosGroup/Vulkan-Headers.git"\nSCRIPT_COMMIT="v1.4.366"\n`);
  writeFileSync(join(d, "47-vulkan", "99-enable.sh"), `SCRIPT_SKIP="1"\nffbuild_configure() {\n    echo --enable-vulkan\n}\n`);
  return d;
}

describe("parseScript", () => {
  it("reads the pinned source, the stages it needs and the flags it turns on", () => {
    const s = parseScript(AOM, "scripts.d/50-aom.sh");
    expect(s.name).toBe("aom");
    expect(s.sources).toEqual([{ repo: "https://aomedia.googlesource.com/aom", commit: "9ace39e02a34992b83fc7f7f2bfbbd2e79348c74", branch: null, n: "1" }]);
    expect(s.depends).toEqual(["base", "vmaf"]);
    expect(s.flags).toEqual(["libaom"]);
  });

  it("does not take --disable-* or the unconfigure function for a flag", () => {
    expect(parseScript(AOM, "x.sh").flags).not.toContain("disable-libaom");
  });
});

describe("stagesFor", () => {
  it("takes the stages that turn the flags on and everything they depend on", () => {
    const stages = loadStages(scriptsDir());
    const r = stagesFor(stages, ["libaom", "gpl", "vulkan"], { options: ["gpl"] });
    expect(r.stages).toEqual(["aom", "vmaf", "vulkan"]);
    expect(r.unclaimed).toEqual([]);
  });

  it("reports a flag no script claims instead of dropping it", () => {
    const stages = loadStages(scriptsDir());
    expect(stagesFor(stages, ["libaom", "mystery"]).unclaimed).toEqual(["mystery"]);
  });

  it("leaves out a library the binary does not enable", () => {
    const stages = loadStages(scriptsDir());
    expect(stagesFor(stages, ["libaom"]).stages).not.toContain("unused");
  });

  it("lets a Windows interface go without a source", () => {
    const stages = loadStages(scriptsDir());
    expect(stagesFor(stages, ["schannel"], { system: ["schannel"] })).toEqual({ stages: [], unclaimed: [] });
  });
});

describe("archiveUrl", () => {
  it("knows GitHub, GitLab and googlesource", () => {
    expect(archiveUrl("https://github.com/Netflix/vmaf.git", "abc")).toEqual({ url: "https://github.com/Netflix/vmaf/archive/abc.tar.gz", file: "vmaf-abc.tar.gz" });
    expect(archiveUrl("https://code.videolan.org/videolan/libplacebo.git", "abc").url).toBe("https://code.videolan.org/videolan/libplacebo/-/archive/abc/libplacebo-abc.tar.gz");
    expect(archiveUrl("https://aomedia.googlesource.com/aom", "abc").url).toBe("https://aomedia.googlesource.com/aom/+archive/abc.tar.gz");
  });

  it("knows SourceForge git, which names its repository after the project path", () => {
    expect(archiveUrl("https://git.code.sf.net/p/opencore-amr/code", "abc").url).toBe("https://sourceforge.net/p/opencore-amr/code/ci/abc/tarball");
    expect(archiveUrl("https://git.code.sf.net/p/mingw-w64/mingw-w64.git", "abc").url).toBe("https://sourceforge.net/p/mingw-w64/mingw-w64/ci/abc/tarball");
  });

  it("says null for a host it does not know", () => {
    expect(archiveUrl("https://example.org/x.git", "abc")).toBeNull();
  });
});

describe("componentsFor", () => {
  const make = (wanted) => componentsFor(loadStages(scriptsDir()), wanted, { scriptsCommit: "c63bb308" });

  it("gives each flag its own component over the same file", () => {
    const c = make(["ffnvcodec"]);
    expect(c.map((x) => x.flag)).toEqual(["ffnvcodec", "cuda-llvm"]);
    expect(new Set(c.map((x) => x.file)).size).toBe(1);
  });

  it("ignores the older SDKs a script lists but this build does not use", () => {
    expect(make(["ffnvcodec"]).every((x) => x.exact === "f2053071ea93137255b6dcf42e9b4c9e0965a44e")).toBe(true);
  });

  it("says why a subversion checkout has no URL", () => {
    const [c] = make(["libmp3lame"]);
    expect(c.url).toBeNull();
    expect(c.note).toContain("7113");
  });

  it("leaves a stage that is only a switch (SCRIPT_SKIP) out", () => {
    expect(make(["vulkan"]).map((x) => x.name)).toEqual(["vulkan-headers"]);
  });
});
