// `node scripts/release/channels.mjs <version> <installer-name> <installer.sig>` — the release
// workflow's last step (.github/workflows/release.yml): once the tagged release is published, point
// the channel manifests at it. Needs `gh` signed in (GH_TOKEN) and the version's section in
// docs/CHANGELOG.md. Never builds or uploads an installer.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseVersion } from "./lib.mjs";
import { ghCli, updateChannels } from "./publish.mjs";
import { notesFor } from "./release.mjs";
import { ROOT } from "./set-version.mjs";

async function main([version, installer, sigFile]) {
  if (!version || !installer || !sigFile) {
    console.error("usage: node scripts/release/channels.mjs 26.X.Y-alpha|beta|stable <installer-name> <installer.sig>");
    return 2;
  }
  parseVersion(version);
  const plan = {
    version,
    notes: notesFor(readFileSync(join(ROOT, "docs/CHANGELOG.md"), "utf8"), version),
    files: [{ name: installer }],
    signature: readFileSync(sigFile, "utf8").trim(),
    now: new Date().toISOString(),
  };
  await updateChannels(ghCli(), plan);
  console.log(`Channels now offer Lumen ${version}.`);
  return 0;
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
