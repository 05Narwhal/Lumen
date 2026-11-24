// `node scripts/release/channels.mjs <version> <installer-name> <installer.sig>` — the release
// workflow's last step (.github/workflows/release.yml): once the tagged release is published, point
// the channel manifests at it. Needs `gh` signed in (GH_TOKEN) and the version's section in
// docs/CHANGELOG.md. Never builds or uploads an installer.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { isSecurityRelease, notesFor, parseVersion } from "./lib.mjs";
import { ghCli, updateChannels } from "./publish.mjs";
import { ROOT } from "./set-version.mjs";

/**
 * What the manifests are written from. `security` is the changelog heading's `· security` marker:
 * it sets `minSafe` in the channels this build reaches.
 */
export function releasePlan({ version, changelog, installer, signature, now }) {
  parseVersion(version);
  return { version, notes: notesFor(changelog, version), security: isSecurityRelease(changelog, version), files: [{ name: installer }], signature: signature.trim(), now };
}

async function main([version, installer, sigFile]) {
  if (!version || !installer || !sigFile) {
    console.error("usage: node scripts/release/channels.mjs 26.X.Y-alpha|beta|stable <installer-name> <installer.sig>");
    return 2;
  }
  const plan = releasePlan({ version, changelog: readFileSync(join(ROOT, "docs/CHANGELOG.md"), "utf8"), installer, signature: readFileSync(sigFile, "utf8"), now: new Date().toISOString() });
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
