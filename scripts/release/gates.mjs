// The release gates (Round 17, spec "Release script and gates"). Each is a pure function of facts
// that `collect.mjs` gathers, so the rules are tested without git, cargo or GitHub. A release
// goes out only when every gate holds.
import { isNextVersion, parseVersion } from "./lib.mjs";

/** What `ffmpeg -version` says: its version and every `--enable-*` it was configured with. */
export function parseFfmpegVersion(output) {
  // gyan.dev: `9.0.1-essentials…` → 9.0.1; BtbN: `n9.0.2-47-gb204e94e90-20261106` → as written.
  const version = /^ffmpeg version (n?\d+\.\d+(?:\.\d+)?(?:-\d+-g[0-9a-f]+-\d+)?)/m.exec(output ?? "")?.[1] ?? null;
  const config = /^configuration: (.*)$/m.exec(output ?? "")?.[1] ?? "";
  const enabled = [...config.matchAll(/--enable-([a-z0-9-]+)/g)].map((m) => m[1]);
  return { version, enabled };
}

/**
 * FFmpeg is GPL-3.0 here (it carries x264 and x265), so every library in the shipped binary must
 * have its exact source in the release's source bundle. The rule reads the binary itself: a new
 * library in a new build, without its source listed, refuses the release.
 */
export function ffmpegGate({ versionOutput, sidecarSha256 }, sources, bundle) {
  const why = [];
  if (!versionOutput) return { ok: false, why: ["the FFmpeg sidecar could not be asked its version"] };
  const { version, enabled } = parseFfmpegVersion(versionOutput);
  if (version !== sources.version) why.push(`the sidecar is FFmpeg ${version}, sources.json records ${sources.version}`);
  if (sidecarSha256 !== sources.sidecarSha256) why.push("the sidecar is not the build sources.json records (SHA-256 differs)");
  const options = new Set(sources.options ?? []);
  const byFlag = new Map(sources.components.filter((c) => c.flag).map((c) => [c.flag, c]));
  for (const flag of enabled) {
    if (options.has(flag)) continue;
    const c = byFlag.get(flag);
    if (!c) why.push(`--enable-${flag} has no source recorded`);
  }
  for (const c of sources.components) {
    if (c.os) continue;
    if (c.flag && !enabled.includes(c.flag)) continue;
    if ((!c.url && !c.local) || !c.sha256 || !c.file) {
      why.push(`${c.name}: no source URL and SHA-256 recorded`);
      continue;
    }
    if (!(c.file in bundle)) why.push(`${c.name}: ${c.file} is not in the source bundle`);
    else if (bundle[c.file] !== c.sha256) why.push(`${c.name}: ${c.file} in the bundle has the wrong SHA-256`);
  }
  return { ok: why.length === 0, why };
}

/**
 * The sources a release carries besides FFmpeg's: ExifTool and its Perl (GPL / Artistic). The
 * photos-raw library's source is the tagged public repository, so it is not archived here.
 */
export function sourceBundleGate(others, bundle) {
  const why = [];
  for (const s of others) {
    if (!s.file || !s.url || !s.sha256) why.push(`${s.name}: no source URL and SHA-256 recorded`);
    else if (!(s.file in bundle)) why.push(`${s.name}: ${s.file} is not in the source bundle`);
    else if (bundle[s.file] !== s.sha256) why.push(`${s.name}: ${s.file} in the bundle has the wrong SHA-256`);
  }
  return { ok: why.length === 0, why };
}

/**
 * The packages `cargo tree -i rawler --depth 1` lists as depending on rawler: `[]` when no package
 * uses rawler, `null` when the output can't be read (a failed call must never pass as "no rawler").
 */
export function rawlerDependents(cargoTreeOutput) {
  const text = cargoTreeOutput ?? "";
  if (/package ID specification `rawler` did not match any packages/.test(text)) return [];
  const lines = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  if (!lines.length || !lines[0].startsWith("rawler ")) return null;
  return lines.slice(1).map((l) => l.split(" ")[0]);
}

const MB = 1024 * 1024;
/** Lumen Setup's size budget (spec "Size budget"): the full one carries the installer plus 6 MB; the slim one is 6 MB. */
export const SETUP_SLIM_MAX = 6 * MB;
export const SETUP_FULL_EXTRA = 6 * MB;

const mb = (n) => `${(n / MB).toFixed(2)} MB`;

/** Both Setup sizes against the budget. `nsis`, `full`, `slim` are byte counts of the built files. */
export function setupSizeGate({ nsis, full, slim } = {}) {
  const why = [];
  const bad = (n) => !Number.isFinite(n) || n <= 0;
  if (bad(nsis) || bad(full) || bad(slim)) return { ok: false, why: ["the sizes of the installer and both Setups could not be read"] };
  if (full > nsis + SETUP_FULL_EXTRA) why.push(`the full Setup is ${mb(full)}, over the installer's ${mb(nsis)} + 6 MB`);
  if (slim > SETUP_SLIM_MAX) why.push(`the slim Setup is ${mb(slim)}, over 6 MB`);
  return { ok: why.length === 0, why };
}

const RAW_LIBRARY = /^(lib)?photos_raw\.(dll|so|dylib)$/;

/** Every gate, in the order a person reads them. `why` never carries a secret. */
export function evaluateGates(f, { version }) {
  const gates = [];
  const gate = (id, ok, why) => gates.push({ id, ok: !!ok, why: ok ? "" : why });
  let channel = null;
  try {
    channel = parseVersion(version).channel;
  } catch (e) {
    gate("version", false, e.message);
  }
  gate("clean", f.clean, "The working tree has changes. Commit or remove them first.");
  gate("main", f.branch === "main", `Releases are made from main, not ${f.branch}.`);
  gate("changelog", f.changelogVersions?.includes(version), `docs/CHANGELOG.md has no "## ${version}" entry.`);
  gate("version-fields", f.versionFields === version, `The version fields say ${f.versionFields}. Run node scripts/release/set-version.mjs ${version} and commit.`);
  if (channel) {
    let next = false;
    try {
      next = isNextVersion(f.previousVersion, version);
    } catch {
      next = false;
    }
    gate("next-version", next, `${version} does not follow the last release, ${f.previousVersion}. Every build gets a new number.`);
  }
  const missing = Object.entries(f.sidecars ?? {}).filter(([, here]) => !here).map(([name]) => name);
  gate("sidecars", missing.length === 0 && f.sidecars, `Missing in src-tauri/binaries: ${missing.join(", ")}.`);
  gate("notices", f.noticesFresh, "src/assets/licenses/notices.json changed when regenerated. Commit yarn run licenses first.");
  gate("checks", f.checks?.ok, `Checks failed: ${(f.checks?.failed ?? []).join(", ")}.`);
  gate(
    "signing",
    f.pubkey?.trim() && f.privateKeyInEnv,
    !f.pubkey?.trim() ? "tauri.conf.json has no updater public key." : "TAURI_SIGNING_PRIVATE_KEY is not set in this terminal.",
  );
  gate("no-test-overrides", !(f.testOverrides ?? []).length, `Test-only settings are set: ${(f.testOverrides ?? []).join(", ")}. Unset them before building a release.`);
  gate("library-check", f.libraryCheck?.ok, `The candidate did not open a copy of a real library: ${f.libraryCheck?.why ?? "not run"}.`);
  const sizes = setupSizeGate(f.setupSizes);
  gate("setup-sizes", sizes.ok, `Lumen Setup is over its size budget: ${sizes.why.join("; ")}.`);
  const deps = f.rawler?.dependents;
  const known = Array.isArray(deps);
  const onlyRaw = known && deps.every((d) => d === "photos-raw");
  const bundled = known && (deps.length === 0 || (f.rawler?.resources ?? []).some((r) => RAW_LIBRARY.test(r.split(/[\\/]/).pop())));
  gate(
    "rawler",
    onlyRaw && bundled,
    !known
      ? "cargo tree could not say which packages use rawler."
      : !onlyRaw
        ? `rawler (LGPL-2.1) is linked into ${deps.filter((d) => d !== "photos-raw").join(", ")}; it may only be in the replaceable photos-raw library.`
        : "photos-raw's library is not in tauri.conf.json's bundle.resources.",
  );
  const ff = f.ffmpeg && f.sources?.ffmpeg ? ffmpegGate(f.ffmpeg, f.sources.ffmpeg, f.sourceBundle ?? {}) : { ok: false, why: ["no FFmpeg facts"] };
  gate("ffmpeg-source", ff.ok, `FFmpeg's terms are not met: ${ff.why.join("; ")}.`);
  const others = sourceBundleGate(f.sources?.others ?? [], f.sourceBundle ?? {});
  gate("source-bundle", others.ok, `The source bundle is incomplete: ${others.why.join("; ")}.`);
  gate("releases-repo", f.releasesRepo, "github.com/05Narwhal/lumen does not exist yet, or gh cannot see it (Julian creates it).");
  if (channel === "stable") {
    const open = ["jamendo", "esri"].filter((k) => !f.gates?.[k]?.resolved);
    gate("stable-only", open.length === 0, `Before a stable release: ${open.join(" and ")} still unresolved in scripts/release/gates.json.`);
  }
  return gates;
}
