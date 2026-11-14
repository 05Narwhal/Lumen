import { describe, expect, it } from "vitest";
import { channelManifest, placeholderManifest, publish, updateChannels } from "./publish.mjs";

/** A GitHub that records what was asked of it, in order. */
function fakeGh({ channelsExists = true, verifyFails = false, publishFails = false } = {}) {
  const calls = [];
  let uploaded = [];
  const gh = {
    calls,
    releaseExists: async (tag) => {
      calls.push(["exists", tag]);
      return tag === "channels" ? channelsExists : false;
    },
    createRelease: async (tag, opts) => calls.push(["create", tag, opts.draft, opts.prerelease]),
    upload: async (tag, files) => {
      uploaded = files;
      calls.push(["upload", tag, files.map((f) => f.name)]);
    },
    downloadedHashes: async (tag) => {
      calls.push(["download", tag]);
      return Object.fromEntries(uploaded.map((f, i) => [f.name, verifyFails && i === 0 ? "different" : f.sha256]));
    },
    publishRelease: async (tag) => {
      calls.push(["publish", tag]);
      if (publishFails) throw new Error("GitHub said no");
    },
    uploadChannel: async (name, json) => calls.push(["channel", name, json.version]),
  };
  return gh;
}

const plan = (version = "26.1.0-alpha") => ({
  version,
  notes: "First alpha.",
  files: [
    { name: `Lumen_${version}_x64-setup.exe`, path: "x", sha256: "h1" },
    { name: `Lumen_${version}_x64-setup.exe.sig`, path: "y", sha256: "h2" },
  ],
  signature: "c2ln",
  now: "2026-11-01T12:00:00Z",
});

describe("publish", () => {
  it("drafts, verifies, publishes, and writes the channels last", async () => {
    const gh = fakeGh();
    await publish(gh, plan());
    const kinds = gh.calls.map((c) => c[0]).filter((k) => k !== "exists");
    expect(kinds).toEqual(["create", "upload", "download", "publish", "channel"]);
    expect(gh.calls.find((c) => c[0] === "create")).toEqual(["create", "v26.1.0-alpha", true, true]);
  });

  it("never publishes what did not download back the same", async () => {
    const gh = fakeGh({ verifyFails: true });
    await expect(publish(gh, plan())).rejects.toThrow(/does not match/);
    expect(gh.calls.some((c) => c[0] === "publish" || c[0] === "channel")).toBe(false);
  });

  it("never touches a channel when publishing failed", async () => {
    const gh = fakeGh({ publishFails: true });
    await expect(publish(gh, plan())).rejects.toThrow();
    expect(gh.calls.some((c) => c[0] === "channel")).toBe(false);
  });

  it("writes all three channels the first time, so every manifest exists", async () => {
    const gh = fakeGh({ channelsExists: false });
    await publish(gh, plan());
    expect(gh.calls.filter((c) => c[0] === "channel")).toEqual([
      ["channel", "alpha", "26.1.0-alpha"],
      ["channel", "beta", "0.0.0-beta"],
      ["channel", "stable", "0.0.0-stable"],
    ]);
    expect(gh.calls.find((c) => c[0] === "create" && c[1] === "channels")).toBeTruthy();
  });

  it("an alpha touches only alpha; a stable touches all three", async () => {
    const a = fakeGh();
    await publish(a, plan("26.1.0-alpha"));
    expect(a.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["alpha"]);
    const s = fakeGh();
    await publish(s, plan("26.2.0-stable"));
    expect(s.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["stable", "beta", "alpha"]);
    expect(s.calls.find((c) => c[0] === "create" && c[1] === "v26.2.0-stable")[3]).toBe(false);
  });
});

describe("manifests", () => {
  it("is the updater's static format, with the signature's text", () => {
    expect(channelManifest(plan())).toEqual({
      version: "26.1.0-alpha",
      notes: "First alpha.",
      pub_date: "2026-11-01T12:00:00Z",
      platforms: {
        "windows-x86_64": { signature: "c2ln", url: "https://github.com/05Narwhal/lumen/releases/download/v26.1.0-alpha/Lumen_26.1.0-alpha_x64-setup.exe" },
      },
    });
  });

  it("keeps pointing at the NSIS installer when Lumen Setup is uploaded beside it", () => {
    const withSetup = plan();
    withSetup.files = [{ name: "Lumen-Setup-26.1.0-alpha.exe", sha256: "s" }, ...withSetup.files];
    expect(channelManifest(withSetup).platforms["windows-x86_64"].url).toMatch(/Lumen_26\.1\.0-alpha_x64-setup\.exe$/);
  });

  it("an empty channel offers nothing anyone would take", () => {
    expect(placeholderManifest("stable", "2026-11-01T12:00:00Z")).toEqual({ version: "0.0.0-stable", notes: "", pub_date: "2026-11-01T12:00:00Z", platforms: {} });
  });
});

describe("updateChannels (the release workflow's manifest step)", () => {
  it("writes only the manifests, and makes no release of the version itself", async () => {
    const gh = fakeGh();
    await updateChannels(gh, plan("26.2.0-beta"));
    expect(gh.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["beta", "alpha"]);
    expect(gh.calls.some((c) => ["create", "upload", "publish"].includes(c[0]))).toBe(false);
  });

  it("creates the channels release and all three manifests the first time", async () => {
    const gh = fakeGh({ channelsExists: false });
    await updateChannels(gh, plan());
    expect(gh.calls.find((c) => c[0] === "create")).toEqual(["create", "channels", false, false]);
    expect(gh.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["alpha", "beta", "stable"]);
  });
});
