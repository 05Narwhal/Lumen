import { describe, expect, it } from "vitest";
import { channelManifest, placeholderManifest, updateChannels } from "./publish.mjs";

/** A GitHub that records what was asked of it, in order. */
function fakeGh({ channelsExists = true, manifests = {}, failRead = null } = {}) {
  const calls = [];
  const written = {};
  const gh = {
    calls,
    written,
    readChannel: async (name) => {
      if (name === failRead) throw new Error(`could not read ${name}.json`);
      return manifests[name] ?? null;
    },
    releaseExists: async (tag) => {
      calls.push(["exists", tag]);
      return tag === "channels" ? channelsExists : false;
    },
    createRelease: async (tag, opts) => calls.push(["create", tag, opts.draft, opts.prerelease]),
    upload: async (tag, files) => calls.push(["upload", tag, files.map((f) => f.name)]),
    publishRelease: async (tag) => calls.push(["publish", tag]),
    uploadChannel: async (name, json) => {
      written[name] = json;
      calls.push(["channel", name, json.version]);
    },
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

  it("an alpha touches only alpha; a stable touches all three", async () => {
    const a = fakeGh();
    await updateChannels(a, plan("26.1.0-alpha"));
    expect(a.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["alpha"]);
    const s = fakeGh();
    await updateChannels(s, plan("26.2.0-stable"));
    expect(s.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["stable", "beta", "alpha"]);
  });

  it("creates the channels release and all three manifests the first time", async () => {
    const gh = fakeGh({ channelsExists: false });
    await updateChannels(gh, plan());
    expect(gh.calls.find((c) => c[0] === "create")).toEqual(["create", "channels", false, false]);
    expect(gh.calls.filter((c) => c[0] === "channel").map((c) => c[1])).toEqual(["alpha", "beta", "stable"]);
  });
});

describe("never lower a channel", () => {
  const offering = (version) => ({ version, notes: "n", pub_date: "2026-10-01T00:00:00Z", platforms: { "windows-x86_64": { signature: "old", url: `https://example/${version}` } } });
  const written = (gh) => gh.calls.filter((c) => c[0] === "channel").map((c) => c[1]);

  it("a stable hotfix published after a newer alpha leaves alpha.json and a higher beta.json alone", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, plan("26.3.5-stable"));
    expect(written(gh)).toEqual(["stable"]);
    expect(gh.written.stable.version).toBe("26.3.5-stable");
  });

  it("still moves a channel that is behind the hotfix", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.3.2-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, plan("26.3.5-stable"));
    expect(written(gh)).toEqual(["stable", "alpha"]);
    expect(gh.written.alpha.version).toBe("26.3.5-stable");
  });

  it("a placeholder, a missing manifest and a missing channels release are all replaced", async () => {
    const gh = fakeGh({ manifests: { beta: placeholderManifest("beta", "2026-10-01T00:00:00Z") } });
    await updateChannels(gh, plan("26.1.0-stable"));
    expect(written(gh)).toEqual(["stable", "beta", "alpha"]);
  });

  it("an alpha never touches beta or stable, whatever they hold", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.1-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, plan("26.4.2-alpha"));
    expect(written(gh)).toEqual(["alpha"]);
  });

  it("running the same version again finishes a half-done update", async () => {
    const gh = fakeGh({ manifests: { stable: offering("26.3.5-stable"), beta: offering("26.3.4-stable"), alpha: offering("26.3.4-stable") } });
    await updateChannels(gh, plan("26.3.5-stable"));
    expect(written(gh)).toEqual(["stable", "beta", "alpha"]);
  });

  it("if a manifest cannot be read nothing is written, rather than risk lowering it", async () => {
    const gh = fakeGh({ failRead: "beta", manifests: { stable: offering("26.3.4-stable") } });
    await expect(updateChannels(gh, plan("26.3.5-stable"))).rejects.toThrow(/beta\.json/);
    expect(written(gh)).toEqual([]);
  });
});

describe("minSafe (the security track)", () => {
  const offering = (version, minSafe) => ({
    version,
    notes: "n",
    pub_date: "2026-10-01T00:00:00Z",
    platforms: { "windows-x86_64": { signature: "old", url: `https://example/${version}` } },
    ...(minSafe ? { minSafe } : {}),
  });
  const security = (version) => ({ ...plan(version), security: true });

  it("a security release sets minSafe in every manifest it reaches", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.3.3-alpha"), beta: offering("26.3.3-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, security("26.3.5-stable"));
    for (const name of ["stable", "beta", "alpha"]) expect(gh.written[name].minSafe).toBe("26.3.5-stable");
  });

  it("a security release sets only the channels it reaches", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.1-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, security("26.4.2-alpha"));
    expect(Object.keys(gh.written)).toEqual(["alpha"]);
    expect(gh.written.alpha.minSafe).toBe("26.4.2-alpha");
  });

  it("a channel kept at a higher build still learns minSafe, and keeps its own build", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, security("26.3.5-stable"));
    expect(gh.written.alpha).toEqual(offering("26.4.2-alpha", "26.3.5-stable"));
    expect(gh.written.beta).toEqual(offering("26.4.0-beta", "26.3.5-stable"));
    expect(gh.written.stable.version).toBe("26.3.5-stable");
  });

  it("a later ordinary release copies minSafe forward", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha", "26.3.5-stable") } });
    await updateChannels(gh, plan("26.4.3-alpha"));
    expect(gh.written.alpha.version).toBe("26.4.3-alpha");
    expect(gh.written.alpha.minSafe).toBe("26.3.5-stable");
  });

  it("minSafe only goes up", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha", "26.4.1-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, security("26.3.5-stable"));
    expect(gh.written.alpha).toBeUndefined(); // already carries a higher minSafe and a higher build: nothing to write
    expect(gh.written.stable.minSafe).toBe("26.3.5-stable");
    expect(gh.written.beta.minSafe).toBe("26.3.5-stable");
  });

  it("an unreadable minSafe in a manifest is ignored, not copied", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha", "soon") } });
    await updateChannels(gh, plan("26.4.3-alpha"));
    expect("minSafe" in gh.written.alpha).toBe(false);
  });

  it("with no security release ever, manifests carry no minSafe at all", async () => {
    const gh = fakeGh({ manifests: { alpha: offering("26.4.2-alpha"), beta: offering("26.4.0-beta"), stable: offering("26.3.4-stable") } });
    await updateChannels(gh, plan("26.4.3-stable"));
    for (const json of Object.values(gh.written)) expect("minSafe" in json).toBe(false);
    expect(channelManifest(plan())).not.toHaveProperty("minSafe");
  });

  it("the first release writes placeholders without minSafe", async () => {
    const gh = fakeGh({ channelsExists: false });
    await updateChannels(gh, security("26.1.0-alpha"));
    expect(gh.written.alpha.minSafe).toBe("26.1.0-alpha");
    expect("minSafe" in gh.written.beta).toBe(false);
  });
});
