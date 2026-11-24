import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { collectArtifacts, findSigntool, installedBytes, lumenExeName, mergePatch, nsisBuildArgs, nsisOverlay, resourceKey, signingFromEnv, signtoolArgs, SLIM_TARGET, sizesJson, tauriSignCommand, treeBytes, updaterKeyFromEnv } from "./setup-build.mjs";

const THUMB = "0123456789abcdef0123456789abcdef01234567";

describe("signingFromEnv", () => {
  it("is null without a certificate: the build goes ahead unsigned", () => {
    expect(signingFromEnv({})).toBeNull();
    expect(signingFromEnv({ LUMEN_SIGN_CERT_SHA1: "  " })).toBeNull();
  });

  it("takes a thumbprint, with or without the spaces the certificate viewer shows", () => {
    expect(signingFromEnv({ LUMEN_SIGN_CERT_SHA1: THUMB }).thumbprint).toBe(THUMB);
    expect(signingFromEnv({ LUMEN_SIGN_CERT_SHA1: THUMB.replace(/(.{4})/g, "$1 ") }).thumbprint).toBe(THUMB);
  });

  it("refuses a value that is not a thumbprint instead of signing with nothing", () => {
    expect(() => signingFromEnv({ LUMEN_SIGN_CERT_SHA1: "my-cert" })).toThrow(/thumbprint/);
  });

  it("uses the timestamp server it is given, else DigiCert's", () => {
    expect(signingFromEnv({ LUMEN_SIGN_CERT_SHA1: THUMB }).timestamp).toBe("http://timestamp.digicert.com");
    expect(signingFromEnv({ LUMEN_SIGN_CERT_SHA1: THUMB, LUMEN_SIGN_TIMESTAMP_URL: "http://t.example" }).timestamp).toBe("http://t.example");
  });
});

describe("signing commands", () => {
  const sign = { thumbprint: THUMB, timestamp: "http://t.example" };

  it("signtool signs with SHA-256 and a timestamp, and carries no secret", () => {
    expect(signtoolArgs(sign, "C:/a b/x.exe")).toEqual(["sign", "/sha1", THUMB, "/fd", "SHA256", "/tr", "http://t.example", "/td", "SHA256", "C:/a b/x.exe"]);
  });

  it("Tauri's signCommand has the %1 placeholder where the file goes", () => {
    const c = tauriSignCommand(sign, "C:/kit/signtool.exe");
    expect(c.cmd).toBe("C:/kit/signtool.exe");
    expect(c.args.at(-1)).toBe("%1");
  });
});

describe("nsisOverlay", () => {
  it("puts the slim Setup at the exact name the uninstall hook looks for, and asks for updater artefacts", () => {
    const o = nsisOverlay({ slimPath: "C:/out/lumen-setup.exe", sign: null });
    expect(o.bundle.resources).toEqual({ "C:/out/lumen-setup.exe": "lumen-setup.exe" });
    expect(SLIM_TARGET).toBe("lumen-setup.exe");
    expect(o.bundle.createUpdaterArtifacts).toBe(true);
    expect(o.bundle.windows).toBeUndefined();
  });

  it("signs the installer inside the build only when a certificate is named", () => {
    const o = nsisOverlay({ slimPath: "x", sign: { thumbprint: THUMB, timestamp: "t" }, signtool: "signtool" });
    expect(o.bundle.windows.signCommand.args.at(-1)).toBe("%1");
  });

  it("merges over the app's resources instead of replacing them", () => {
    const base = { bundle: { resources: { "binaries/raw/*": "raw/" }, windows: { nsis: { installMode: "currentUser" } } } };
    const merged = mergePatch(base, nsisOverlay({ slimPath: "x", sign: null }));
    expect(merged.bundle.resources).toEqual({ "binaries/raw/*": "raw/", x: "lumen-setup.exe" });
    expect(merged.bundle.windows.nsis.installMode).toBe("currentUser");
  });
});

describe("installedBytes", () => {
  let dir;
  const file = (rel, bytes) => {
    const p = join(dir, rel);
    mkdirSync(join(p, ".."), { recursive: true });
    writeFileSync(p, Buffer.alloc(bytes));
    return p;
  };
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "lumen-setup-build-"));
  });
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  it("counts Lumen, the sidecars without their triple, folders and globs, the slim Setup and the uninstaller", () => {
    const srcTauri = join(dir, "src-tauri");
    file("src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe", 1000);
    file("src-tauri/binaries/raw/photos_raw.dll", 200);
    file("src-tauri/binaries/exiftool_files/lib/a.pm", 30);
    file("src-tauri/binaries/exiftool_files/b.txt", 4);
    const lumenExe = file("lumen.exe", 5000);
    const slimPath = file("slim/lumen-setup.exe", 70);
    const config = { bundle: { externalBin: ["binaries/ffmpeg"], resources: { "binaries/raw/*": "raw/", "binaries/exiftool_files/": "exiftool_files/", [slimPath]: "lumen-setup.exe" } } };
    expect(installedBytes({ srcTauri, config, lumenExe, slimPath })).toBe(5000 + 128 * 1024 + 70 + 1000 + 200 + 34);
  });

  it("refuses a resource that is missing rather than baking too small a size", () => {
    const srcTauri = join(dir, "src-tauri");
    const lumenExe = file("lumen.exe", 1);
    const slimPath = file("slim.exe", 1);
    expect(() => installedBytes({ srcTauri, config: { bundle: { resources: { "binaries/nope/*": "x/" } } }, lumenExe, slimPath })).toThrow(/does not exist/);
  });

  it("treeBytes adds a folder's files", () => {
    file("t/a", 3);
    file("t/sub/b", 4);
    expect(treeBytes(join(dir, "t"))).toBe(7);
  });
});

describe("findSigntool", () => {
  it("is null when neither PATH nor a Windows SDK has one", () => {
    expect(findSigntool({ PATH: "", "ProgramFiles(x86)": join(tmpdir(), "no-such-kits-folder") })).toBeNull();
  });
});

describe("test-identity overlays", () => {
  it("go after ours, each as its own --config", () => {
    expect(nsisBuildArgs("o.json")).toEqual(["build", "--bundles", "nsis", "--config", "o.json"]);
    expect(nsisBuildArgs("o.json", ["t.json"])).toEqual(["build", "--bundles", "nsis", "--config", "o.json", "--config", "t.json"]);
  });
  it("name Lumen's exe by mainBinaryName", () => {
    expect(lumenExeName({})).toBe("lumen.exe");
    expect(lumenExeName({ mainBinaryName: "lumen-test" })).toBe("lumen-test.exe");
  });
});
describe("updaterKeyFromEnv", () => {
  it("reads the signing key's text from the environment CI sets", () => {
    expect(updaterKeyFromEnv({ TAURI_SIGNING_PRIVATE_KEY: "dW50cnVzdGVkIGNvbW1lbnQ6IHRlc3Q=" })).toBe(true);
  });

  it("refuses a missing key before anything is built, naming the variable", () => {
    expect(() => updaterKeyFromEnv({})).toThrow(/TAURI_SIGNING_PRIVATE_KEY/);
    expect(() => updaterKeyFromEnv({ TAURI_SIGNING_PRIVATE_KEY: "  " })).toThrow(/TAURI_SIGNING_PRIVATE_KEY/);
  });

  it("never reads the key from a file path on a PC", () => {
    for (const path of ["C:/Users/someone/.lumen-release/lumen.key", "C:\\keys\\lumen.key", "/opt/keys/lumen.key", "~/.lumen-release/lumen.key", "lumen.key"]) {
      expect(() => updaterKeyFromEnv({ TAURI_SIGNING_PRIVATE_KEY: path })).toThrow(/text, not a file path/);
    }
    expect(() => updaterKeyFromEnv({ TAURI_SIGNING_PRIVATE_KEY: "dW50cnVzdGVk", TAURI_SIGNING_PRIVATE_KEY_PATH: "C:/k/lumen.key" })).toThrow(/TAURI_SIGNING_PRIVATE_KEY_PATH/);
  });
});

describe("collectArtifacts", () => {
  let dir;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "lumen-artifacts-"));
  });
  afterEach(() => rmSync(dir, { recursive: true, force: true }));
  const put = (name, bytes = 1) => writeFileSync(join(dir, name), Buffer.alloc(bytes));

  it("finds both installers and the .sig in the folder the upload step globs", () => {
    put("Lumen_26.2.1-alpha_x64-setup.exe", 50);
    put("Lumen_26.2.1-alpha_x64-setup.exe.sig", 3);
    put("Lumen-Setup-26.2.1-alpha.exe", 56);
    put("Lumen_26.2.0-alpha_x64-setup.exe", 49);
    const a = collectArtifacts(dir, "26.2.1-alpha");
    expect([a.installer, a.sig, a.full].map((p) => p.split(/[\\/]/).pop())).toEqual(["Lumen_26.2.1-alpha_x64-setup.exe", "Lumen_26.2.1-alpha_x64-setup.exe.sig", "Lumen-Setup-26.2.1-alpha.exe"]);
    expect(a.uploadGlobs).toEqual(["*_26.2.1-alpha_*-setup.exe*", "Lumen-Setup-26.2.1-alpha.exe"]);
  });

  it("refuses an installer without its .sig, because the updater could never take it", () => {
    put("Lumen_26.2.1-alpha_x64-setup.exe");
    put("Lumen-Setup-26.2.1-alpha.exe");
    expect(() => collectArtifacts(dir, "26.2.1-alpha")).toThrow(/\.sig/);
  });

  it("refuses a missing full Setup", () => {
    put("Lumen_26.2.1-alpha_x64-setup.exe");
    put("Lumen_26.2.1-alpha_x64-setup.exe.sig");
    expect(() => collectArtifacts(dir, "26.2.1-alpha")).toThrow(/Lumen-Setup-26\.2\.1-alpha\.exe/);
  });
});

describe("sizesJson", () => {
  it("is the three byte counts the setup-sizes gate reads", () => {
    expect(sizesJson({ sizes: { nsis: 1, full: 2, slim: 3 }, other: 9 })).toEqual({ nsis: 1, full: 2, slim: 3 });
  });
});

describe("resourceKey", () => {
  it("is relative to src-tauri with forward slashes (tauri-build drops an absolute path's drive)", () => {
    expect(resourceKey("X:/repo/src-tauri", "X:/repo/dist-release/26.1.0-alpha/lumen-setup.exe")).toBe("../dist-release/26.1.0-alpha/lumen-setup.exe");
  });
  // `relative()` knows drives only on Windows, which is the only place the release is built.
  it.runIf(process.platform === "win32")("refuses a file on another drive", () => {
    expect(() => resourceKey("X:/repo/src-tauri", "C:/temp/lumen-setup.exe")).toThrow(/same drive/);
  });
});