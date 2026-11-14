// `node scripts/release/release.mjs 26.1.0-alpha [--dry-run] [--skip-checks]` (Round 17; Julian
// runs it, see docs/RELEASING.md). It refuses unless every gate holds, builds the signed NSIS
// installer and Lumen Setup, checks them (a temp copy of a real library, the size budget), then publishes: draft → verify →
// publish → channel manifests last. --dry-run stops before building and publishes nothing.
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { collectFacts, libraryCheck } from "./collect.mjs";
import { evaluateGates } from "./gates.mjs";
import { parseVersion } from "./lib.mjs";
import { ghCli, publish } from "./publish.mjs";
import { ROOT } from "./set-version.mjs";
import { buildInstallers } from "./setup-build.mjs";
import { makeSourceBundle } from "./source-bundle.mjs";

/** Gates that need the built candidate; they run after the build. */
const AFTER_BUILD = ["library-check", "setup-sizes"];

const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

function report(gates) {
  for (const g of gates) console.log(`${g.ok ? "  ok  " : "  NO  "} ${g.id}${g.ok ? "" : `: ${g.why}`}`);
  return gates.every((g) => g.ok);
}

/** This version's notes: its section of docs/CHANGELOG.md. */
export function notesFor(changelog, version) {
  const at = changelog.indexOf(`## ${version}`);
  if (at < 0) return "";
  const rest = changelog.slice(at).split(/\r?\n/).slice(1);
  const end = rest.findIndex((l) => l.startsWith("## "));
  return (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
}

async function main(argv) {
  const version = argv.find((a) => !a.startsWith("--"));
  const dryRun = argv.includes("--dry-run");
  const skipChecks = argv.includes("--skip-checks");
  if (!version) {
    console.error("usage: node scripts/release/release.mjs 26.X.Y-alpha|beta|stable [--dry-run] [--skip-checks]");
    return 2;
  }
  if (skipChecks && !dryRun) {
    console.error("--skip-checks is for --dry-run only.");
    return 2;
  }
  parseVersion(version);
  const out = join(ROOT, "dist-release", version);
  const sourceDir = join(out, "source");
  console.log(`Lumen ${version}: gathering the third-party source…`);
  const sources = JSON.parse(readFileSync(join(ROOT, "scripts/release/sources.json"), "utf8"));
  const missing = await makeSourceBundle(ROOT, sources, sourceDir);
  for (const m of missing) console.log(`        source: ${m}`);
  console.log("Checking the gates…");
  const facts = collectFacts(ROOT, { version, skipChecks, sourceDir });
  const before = evaluateGates(facts, { version }).filter((g) => !AFTER_BUILD.includes(g.id));
  const ready = report(before);
  if (!ready) {
    console.log("\nRefused: nothing was built or published.");
    return 1;
  }
  if (dryRun) {
    console.log("\nEvery gate holds. --dry-run: nothing was built or published.");
    return 0;
  }
  console.log("Building the installer and Lumen Setup…");
  const built = buildInstallers({ root: ROOT, version, out });
  const exe = join(ROOT, "src-tauri/target/release/lumen.exe");
  facts.libraryCheck = libraryCheck(exe, process.env.LUMEN_RELEASE_LIBRARY);
  facts.setupSizes = built.sizes;
  if (!report(evaluateGates(facts, { version }).filter((g) => AFTER_BUILD.includes(g.id)))) return 1;
  if (!built.signed) console.log("\nNot signed: Windows SmartScreen will warn when people open Lumen-Setup and the installer. Set LUMEN_SIGN_CERT_SHA1 to sign.");
  const nsis = dirname(built.installer);
  const installer = built.installerName;
  if (!existsSync(join(nsis, `${installer}.sig`))) {
    console.error(`No updater signature for ${installer} in ${nsis}.`);
    return 1;
  }
  const files = [installer, `${installer}.sig`, built.fullName].map((n) => ({ name: n, path: join(nsis, n), sha256: sha256(join(nsis, n)) }));
  for (const n of readdirSync(sourceDir)) files.push({ name: n, path: join(sourceDir, n), sha256: sha256(join(sourceDir, n)) });
  const plan = {
    version,
    notes: notesFor(readFileSync(join(ROOT, "docs/CHANGELOG.md"), "utf8"), version),
    files,
    signature: readFileSync(join(nsis, `${installer}.sig`), "utf8").trim(),
    now: new Date().toISOString(),
  };
  console.log("Publishing: draft, verify, publish, channels last…");
  await publish(ghCli(), plan);
  console.log(`Published Lumen ${version}.`);
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
