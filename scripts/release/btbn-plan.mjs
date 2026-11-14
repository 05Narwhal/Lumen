// Offline plan for a BtbN build's source list. Prints which libraries went into the binary, where
// each source is, and what is still unresolved (no URL, subversion, submodules, unclaimed flags).
//   node scripts/release/btbn-plan.mjs <FFmpeg-Builds checkout at the scripts commit> <ffmpeg -version output file> [out.json]
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { componentsFor, loadStages, stagesFor } from "./btbn-sources.mjs";
import { parseFfmpegVersion } from "./gates.mjs";

// Not libraries: build switches. `system`: Windows interfaces that come with the OS.
const OPTIONS = ["gpl", "version3", "pthreads"];
const SYSTEM = ["schannel"];

export function plan(checkout, versionOutput) {
  const { version, enabled } = parseFfmpegVersion(versionOutput);
  const stages = loadStages(join(checkout, "scripts.d"));
  const { stages: names, unclaimed } = stagesFor(stages, enabled, { options: OPTIONS, system: SYSTEM });
  const components = componentsFor(stages, names, {});
  return { version, enabled, stages: names, unclaimed, components };
}

if (import.meta.url === `file://${process.argv[1].replace(/\\/g, "/")}` || process.argv[1]?.endsWith("btbn-plan.mjs")) {
  const [checkout, versionFile, out] = process.argv.slice(2);
  const p = plan(checkout, readFileSync(versionFile, "utf8"));
  const noUrl = p.components.filter((c) => !c.url);
  console.log(`FFmpeg ${p.version}: ${p.enabled.length} flags, ${p.stages.length} stages, ${p.components.length} components`);
  console.log(`unclaimed flags (no script): ${p.unclaimed.join(", ") || "none"}`);
  console.log(`without a URL (${noUrl.length}):`);
  for (const c of noUrl) console.log(`  ${c.name} [${c.flag ?? "-"}]: ${c.note}`);
  const sub = [...new Set(p.components.filter((c) => c.submodules).map((c) => c.name))];
  console.log(`built with git submodules (their sources are not in a GitHub archive): ${sub.join(", ") || "none"}`);
  if (out) writeFileSync(out, JSON.stringify(p.components, null, 1));
}
