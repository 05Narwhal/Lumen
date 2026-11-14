// The version scheme (Round 17): `26.X.Y-alpha|beta|stable`. X only when Julian says, Y for most
// updates, resetting to 0 on a new X. Every published build gets a new number, promotions
// included, so one number means one build. A plain `26.1.0` is never published: semver ranks it
// above `-stable`.

const SCHEME = /^26\.(0|[1-9]\d*)\.(0|[1-9]\d*)-(alpha|beta|stable)$/;
const STABILITY = ["alpha", "beta", "stable"];

export function parseVersion(version) {
  const m = SCHEME.exec(String(version));
  if (!m) throw new Error(`${version} is not 26.X.Y-alpha|beta|stable`);
  const [x, y] = [Number(m[1]), Number(m[2])];
  return { x, y, channel: m[3], numeric: `26.${x}.${y}` };
}

/** The channels a build reaches: its own and every less stable one (spec, *Publishing order*). */
export function channelsUpdatedBy(channel) {
  const at = STABILITY.indexOf(channel);
  if (at < 0) throw new Error(`no channel ${channel}`);
  return STABILITY.slice(0, at + 1).reverse();
}

/** Semver order for versions in the scheme: by number, then alpha < beta < stable. */
export function compareVersions(a, b) {
  const [p, q] = [parseVersion(a), parseVersion(b)];
  return p.x - q.x || p.y - q.y || STABILITY.indexOf(p.channel) - STABILITY.indexOf(q.channel);
}

/** Whether `next` may follow `prev` (`null` before the first release). */
export function isNextVersion(prev, next) {
  const n = parseVersion(next);
  if (prev === null) return true;
  const p = parseVersion(prev);
  if (n.x === p.x) return n.y > p.y;
  return n.x > p.x && n.y === 0;
}

/**
 * The version fields with `version` written in: `package.json`, `tauri.conf.json`, the workspace
 * `Cargo.toml` and the numeric WiX version (an MSI allows numbers only). Text edits, so nothing
 * else in those files moves.
 */
export function writeVersionFields({ packageJson, tauriConf, cargoToml }, version) {
  const { numeric } = parseVersion(version);
  const once = (text, pattern, value, file) => {
    if (!pattern.test(text)) throw new Error(`no version field in ${file}`);
    return text.replace(pattern, value);
  };
  return {
    packageJson: once(packageJson, /("version":\s*")[^"]*(")/, `$1${version}$2`, "package.json"),
    tauriConf: once(
      once(tauriConf, /("version":\s*")[^"]*(")/, `$1${version}$2`, "tauri.conf.json"),
      /("wix":\s*\{\s*"version":\s*")[^"]*(")/,
      `$1${numeric}$2`,
      "tauri.conf.json (wix)",
    ),
    cargoToml: once(cargoToml, /(\[workspace\.package\]\r?\nversion = ")[^"]*(")/, `$1${version}$2`, "Cargo.toml"),
  };
}

/**
 * Lumen Setup's three version fields (`installer/package.json`, `installer/src-tauri/tauri.conf.json`
 * and `Cargo.toml`'s `[package]`), which must say the version it installs: Setup shows
 * `CARGO_PKG_VERSION` as "Lumen <version>" and compares it with the installed one.
 */
export function writeSetupVersionFields({ packageJson, tauriConf, cargoToml }, version) {
  const once = (text, pattern, value, file) => {
    if (!pattern.test(text)) throw new Error(`no version field in ${file}`);
    return text.replace(pattern, value);
  };
  return {
    packageJson: once(packageJson, /("version":\s*")[^"]*(")/, `$1${version}$2`, "installer/package.json"),
    tauriConf: once(tauriConf, /("version":\s*")[^"]*(")/, `$1${version}$2`, "installer/src-tauri/tauri.conf.json"),
    cargoToml: once(cargoToml, /(\[package\][^[]*?\r?\nversion = ")[^"]*(")/, `$1${version}$2`, "installer/src-tauri/Cargo.toml"),
  };
}
