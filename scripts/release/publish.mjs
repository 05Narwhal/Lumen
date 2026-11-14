// Publishing (Round 17): draft → download back and verify → publish → channel manifests last, so a
// half-done release is never offered. GitHub is passed in (`gh`), so the order is tested with a
// fake; `ghCli()` is the real one, through the `gh` command Julian is signed in to.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { channelsUpdatedBy, parseVersion } from "./lib.mjs";

export const REPO = "05Narwhal/lumen";
const CHANNELS = ["alpha", "beta", "stable"];

const installerUrl = (version, name) => `https://github.com/${REPO}/releases/download/v${version}/${name}`;

/** A channel's manifest, in `tauri-plugin-updater`'s static format. */
export function channelManifest({ version, notes, files, signature, now }) {
  const installer = files.find((f) => f.name.endsWith("-setup.exe"));
  return { version, notes, pub_date: now, platforms: { "windows-x86_64": { signature, url: installerUrl(version, installer.name) } } };
}

/** A channel with no build yet: a version below every real one, so nobody takes it. */
export function placeholderManifest(channel, now) {
  return { version: `0.0.0-${channel}`, notes: "", pub_date: now, platforms: {} };
}

export async function publish(gh, plan) {
  const { version } = plan;
  const { channel } = parseVersion(version);
  const tag = `v${version}`;
  const firstTime = !(await gh.releaseExists("channels"));
  await gh.createRelease(tag, { draft: true, prerelease: channel !== "stable", title: `Lumen ${version}`, notes: plan.notes });
  await gh.upload(tag, plan.files);
  const back = await gh.downloadedHashes(tag);
  for (const f of plan.files) {
    if (back[f.name] !== f.sha256) throw new Error(`${f.name} downloaded from the draft does not match what was uploaded; nothing was published`);
  }
  await gh.publishRelease(tag);
  await writeChannels(gh, plan, firstTime);
}

/**
 * The release workflow's manifest step: the channel manifests only, for a release the workflow
 * has already made and published. The `channels` release is created the first time.
 */
export async function updateChannels(gh, plan) {
  await writeChannels(gh, plan, !(await gh.releaseExists("channels")));
}

async function writeChannels(gh, plan, firstTime) {
  const { channel } = parseVersion(plan.version);
  if (firstTime) await gh.createRelease("channels", { draft: false, prerelease: false, title: "Update channels", notes: "The manifests Lumen reads to find updates. Not an installer." });
  const reached = channelsUpdatedBy(channel);
  const order = firstTime ? CHANNELS : reached;
  for (const name of order) {
    const json = reached.includes(name) ? channelManifest(plan) : placeholderManifest(name, plan.now);
    await gh.uploadChannel(name, json);
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
