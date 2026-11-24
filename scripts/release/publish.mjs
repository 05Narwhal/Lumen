// The channel manifests (Round 17; the release itself is made by CI since release simplification
// phase 2). The workflow publishes the draft and then calls `updateChannels` last, so a half-done
// release is never offered. GitHub is passed in (`gh`), so the order is tested with a fake;
// `ghCli()` is the real one, through the `gh` command (GH_TOKEN in CI).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { channelsUpdatedBy, compareVersions, parseVersion } from "./lib.mjs";

export const REPO = "05Narwhal/lumen";
const CHANNELS = ["alpha", "beta", "stable"];

const installerUrl = (version, name) => `https://github.com/${REPO}/releases/download/v${version}/${name}`;

/**
 * A channel's manifest, in `tauri-plugin-updater`'s static format. `minSafe` is Lumen's own field
 * (the lowest build with no known vulnerability); with none, the manifest is exactly as it was
 * before the security track.
 */
export function channelManifest({ version, notes, files, signature, now, minSafe }) {
  const installer = files.find((f) => f.name.endsWith("-setup.exe"));
  return { version, notes, pub_date: now, platforms: { "windows-x86_64": { signature, url: installerUrl(version, installer.name) } }, ...(minSafe ? { minSafe } : {}) };
}

/** A channel with no build yet: a version below every real one, so nobody takes it. */
export function placeholderManifest(channel, now) {
  return { version: `0.0.0-${channel}`, notes: "", pub_date: now, platforms: {} };
}

/**
 * The release workflow's manifest step: the channel manifests only, for a release the workflow
 * has already made and published. The `channels` release is created the first time.
 */
export async function updateChannels(gh, plan) {
  await writeChannels(gh, plan, !(await gh.releaseExists("channels")));
}

/** Whether a manifest already on a channel offers something the new build may replace: nothing, a placeholder, or an older or equal build. */
function mayReplace(existing, version) {
  if (!existing?.version) return true;
  try {
    return compareVersions(version, existing.version) >= 0;
  } catch {
    return true; // `0.0.0-<channel>`, the placeholder, is not in the scheme
  }
}

/** `minSafe` as a manifest holds it: a version in the scheme, else nothing. */
const validMinSafe = (json) => {
  try {
    parseVersion(json?.minSafe);
    return json.minSafe;
  } catch {
    return null;
  }
};

/** The `minSafe` a channel carries after this release: the highest of what it had and, for a security release, this version. It only goes up. */
function nextMinSafe(existing, plan) {
  const known = [validMinSafe(existing), plan.security ? plan.version : null].filter(Boolean);
  return known.sort(compareVersions).pop() ?? null;
}

async function writeChannels(gh, plan, firstTime) {
  const { channel } = parseVersion(plan.version);
  if (firstTime) await gh.createRelease("channels", { draft: false, prerelease: false, title: "Update channels", notes: "The manifests Lumen reads to find updates. Not an installer." });
  const reached = channelsUpdatedBy(channel);
  // Read every manifest before writing any, so a read that fails stops the whole update. Never lower
  // a channel: a stable hotfix published after a newer alpha must not point alpha.json back at it.
  const existing = Object.fromEntries(await Promise.all(reached.map(async (name) => [name, firstTime ? null : await gh.readChannel(name)])));
  const order = firstTime ? CHANNELS : reached;
  for (const name of order) {
    if (!reached.includes(name)) {
      await gh.uploadChannel(name, placeholderManifest(name, plan.now));
      continue;
    }
    const minSafe = nextMinSafe(existing[name], plan);
    if (mayReplace(existing[name], plan.version)) await gh.uploadChannel(name, channelManifest({ ...plan, minSafe }));
    else if (plan.security && minSafe !== validMinSafe(existing[name])) await gh.uploadChannel(name, { ...existing[name], minSafe });
  }
}

const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

/** GitHub through the `gh` CLI. */
export function ghCli(repo = REPO) {
  const run = (args) => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  return {
    releaseExists: async (tag) => {
      try {
        run(["release", "view", tag, "--repo", repo, "--json", "tagName"]);
        return true;
      } catch {
        return false;
      }
    },
    createRelease: async (tag, { draft, prerelease, title, notes }) => {
      const args = ["release", "create", tag, "--repo", repo, "--title", title, "--notes", notes, "--latest=false"];
      if (draft) args.push("--draft");
      if (prerelease) args.push("--prerelease");
      run(args);
    },
    upload: async (tag, files) => run(["release", "upload", tag, "--repo", repo, "--clobber", ...files.map((f) => f.path)]),
    downloadedHashes: async (tag) => {
      const dir = mkdtempSync(join(tmpdir(), "lumen-release-verify-"));
      try {
        run(["release", "download", tag, "--repo", repo, "--dir", dir]);
        return Object.fromEntries(readdirSync(dir).map((n) => [n, sha256(join(dir, n))]));
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    },
    publishRelease: async (tag) => run(["release", "edit", tag, "--repo", repo, "--draft=false"]),
    // null only when the channels release has no such manifest; any other failure throws, so an
    // unreadable manifest is never mistaken for an empty one and overwritten with a lower build.
    readChannel: async (name) => {
      const assets = JSON.parse(run(["release", "view", "channels", "--repo", repo, "--json", "assets"])).assets ?? [];
      if (!assets.some((a) => a.name === `${name}.json`)) return null;
      return JSON.parse(run(["release", "download", "channels", "--repo", repo, "--pattern", `${name}.json`, "--output", "-"]));
    },
    uploadChannel: async (name, json) => {
      const dir = mkdtempSync(join(tmpdir(), "lumen-channel-"));
      try {
        const file = join(dir, `${name}.json`);
        writeFileSync(file, `${JSON.stringify(json, null, 2)}\n`);
        run(["release", "upload", "channels", "--repo", repo, "--clobber", file]);
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    },
  };
}
