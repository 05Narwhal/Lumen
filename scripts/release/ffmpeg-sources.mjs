// `node scripts/release/ffmpeg-sources.mjs <release-tag>` — gathers the exact source of the FFmpeg
// build Lumen ships (and ExifTool's) from sources.json into a temp folder, checks every SHA-256
// (makeSourceBundle), and uploads it as one GitHub release. The release workflow runs it once per
// FFmpeg build and skips it when the release already exists; the GPL asks for this source, every
// other source is the tagged repository. Needs `gh` signed in (GH_TOKEN).
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { REPO } from "./publish.mjs";
import { ROOT } from "./set-version.mjs";
import { makeSourceBundle } from "./source-bundle.mjs";

async function main([tag]) {
  if (!tag) {
    console.error("usage: node scripts/release/ffmpeg-sources.mjs <release-tag>");
    return 2;
  }
  const sources = JSON.parse(readFileSync(join(ROOT, "scripts/release/sources.json"), "utf8"));
  const dir = mkdtempSync(join(tmpdir(), "lumen-ffmpeg-sources-"));
  try {
    const missing = await makeSourceBundle(ROOT, sources, dir);
    if (missing.length > 0) {
      for (const m of missing) console.error(`source: ${m}`);
      return 1;
    }
    const gh = (args) => execFileSync("gh", args, { stdio: "inherit" });
    gh(["release", "create", tag, "--repo", REPO, "--title", `FFmpeg ${sources.ffmpeg.version}: source`, "--notes", `The exact source of the FFmpeg build ${sources.ffmpeg.version} that Lumen ships, and of ExifTool, with SHA-256s in scripts/release/sources.json. Not an installer.`, "--latest=false"]);
    const files = readdirSync(dir).map((n) => join(dir, n));
    for (let i = 0; i < files.length; i += 50) gh(["release", "upload", tag, "--repo", REPO, "--clobber", ...files.slice(i, i + 50)]);
    return 0;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      console.error(e.message);
      process.exit(1);
    },
  );
}
