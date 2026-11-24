// The release rules that need real logic (Round 17; CI since release simplification phase 2). Each
// is a pure function of facts that `preflight.mjs` gathers, so the rules are tested without git,
// cargo or GitHub. `preflight.mjs` composes them with the simple ones into the list of gates.

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

/** `photos_raw.dll` / `libphotos_raw.so` / `.dylib`, the replaceable library rawler is allowed in. */
export const RAW_LIBRARY = /^(lib)?photos_raw\.(dll|so|dylib)$/;

/** Settings that only a test identity sets: a release build must not see any of them. */
export const TEST_OVERRIDES = ["LUMEN_BAKED_APP_DATA_DIR", "LUMEN_BAKED_LIBRARY_DIR", "LUMEN_UPDATE_TEST_BASE", "LUMEN_SETUP_PRODUCT", "LUMEN_SETUP_IDENTIFIER"];

/**
 * The release tag must be `v<version>` and its commit must be on `main` or a `release/*` branch.
 * `branches` are the branches (no remote prefix) that contain the tagged commit.
 */
export function tagRefGate({ ref, branches }, version) {
  const why = [];
  const name = String(ref ?? "").replace(/^refs\/tags\//, "");
  if (name !== `v${version}`) why.push(`the tag is ${name || "unknown"}, not v${version}`);
  if (!(branches ?? []).some((b) => b === "main" || /^release\/.+/.test(b))) why.push(`its commit is on ${(branches ?? []).join(", ") || "no branch"}, not main or release/*`);
  return { ok: why.length === 0, why };
}
