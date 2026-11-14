import { describe, expect, it } from "vitest";
import { crateRow, gitRepoOf, parseCargoLock, parseGitlinks, parseGitmodules } from "./btbn-fetch.mjs";
import { parseMultistatus } from "./svn-export.mjs";

describe("submodules", () => {
  it("reads .gitmodules and the gitlinks of a tree", () => {
    const mods = parseGitmodules(`[submodule "a"]\n\tpath = third_party/highway\n\turl = https://github.com/google/highway.git\n[submodule "b"]\n\tpath = x\n\turl = ../y.git\n`);
    expect(mods).toEqual([
      { path: "third_party/highway", url: "https://github.com/google/highway.git" },
      { path: "x", url: "../y.git" },
    ]);
    const links = parseGitlinks(`100644 blob abc\tREADME\n160000 commit ${"a".repeat(40)}\tthird_party/highway\n`);
    expect(links).toEqual([{ commit: "a".repeat(40), path: "third_party/highway" }]);
  });
});

describe("Cargo.lock", () => {
  const lock = `[[package]]\nname = "rav1e"\nversion = "0.8.1"\n\n[[package]]\nname = "cc"\nversion = "1.2.3"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "${"b".repeat(64)}"\n`;
  it("lists registry crates and skips workspace members", () => {
    const [c, ...rest] = parseCargoLock(lock);
    expect(rest).toEqual([]);
    expect(crateRow(c, "rav1e")).toMatchObject({ url: "https://static.crates.io/crates/cc/cc-1.2.3.crate", sha256: "b".repeat(64), file: "crate-cc-1.2.3.crate" });
  });
  it("refuses a crate it cannot link", () => {
    expect(() => parseCargoLock(`[[package]]\nname = "x"\nversion = "1"\nsource = "git+https://example.com/x"\n`)).toThrow(/not crates.io/);
  });
});

describe("gitRepoOf", () => {
  it("finds the git URL behind the archive URLs of GitLab, Savannah and GitHub", () => {
    expect(gitRepoOf("https://code.videolan.org/videolan/x264/-/archive/abc/x264-abc.tar.gz")).toBe("https://code.videolan.org/videolan/x264.git");
    expect(gitRepoOf("https://gitlab.freedesktop.org/xorg/lib/libx11/-/archive/abc/libx11-abc.tar.gz")).toBe("https://gitlab.freedesktop.org/xorg/lib/libx11.git");
    expect(gitRepoOf("https://git.savannah.gnu.org/cgit/libiconv.git/snapshot/libiconv-abc.tar.gz")).toBe("https://git.savannah.gnu.org/git/libiconv.git");
    expect(gitRepoOf("https://github.com/GPUOpen-LibrariesAndSDKs/AMF/archive/abc.tar.gz")).toBe("https://github.com/GPUOpen-LibrariesAndSDKs/AMF.git");
    expect(gitRepoOf("https://static.crates.io/crates/cc/cc-1.crate")).toBeNull();
  });
});

describe("svn-export", () => {
  it("reads a PROPFIND answer", () => {
    const xml = `<?xml version="1.0"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/!svn/bc/7/t/</D:href><D:propstat><D:prop><lp1:resourcetype><D:collection/></lp1:resourcetype></D:prop></D:propstat></D:response><D:response><D:href>/!svn/bc/7/t/a%20b.c</D:href><D:propstat><D:prop><lp1:resourcetype/></D:prop></D:propstat></D:response></D:multistatus>`;
    expect(parseMultistatus(xml)).toEqual([
      { href: "/!svn/bc/7/t/", dir: true },
      { href: "/!svn/bc/7/t/a b.c", dir: false },
    ]);
  });
});
