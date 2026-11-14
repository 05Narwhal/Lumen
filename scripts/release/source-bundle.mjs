// The release's third-party source (Round 17): every file sources.json records, downloaded and
// checked against its SHA-256. photos-raw's source is the tagged public repository, so it is not
// archived here. Kept under dist-release/<version>/source/ and uploaded beside the installer.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

const ATTEMPTS = 5;
// A connection that goes quiet must not stall a release for ever (the 400 MB AMF archive is the slowest).
const DOWNLOAD_TIMEOUT_MS = 20 * 60 * 1000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * One file, with the connection dropping part way retried (github resets big downloads: the AMF
 * archive is 398 MB and often needs a few goes). Resolves to the bytes, or throws the last error.
 * An answer that is a plain no (404…) is not retried; it comes back as `{ status }`.
 */
export async function download(url, { fetchFn, retryDelayMs }) {
  let last;
  for (let attempt = 1; attempt <= ATTEMPTS; attempt++) {
    try {
      const res = await fetchFn(url, { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) });
      if (!res.ok) return { status: res.status };
      return { body: Buffer.from(await res.arrayBuffer()) };
    } catch (e) {
      last = e;
      if (attempt < ATTEMPTS) await sleep(retryDelayMs * attempt);
    }
  }
  throw last;
}

const CACHE_DIRS = ["dist-release/ffmpeg/source-cache"];

/** What could not be gathered; an empty list means the folder is complete. */
export async function makeSourceBundle(root, sources, dir, { fetchFn = fetch, retryDelayMs = 2000 } = {}) {
  mkdirSync(dir, { recursive: true });
  const missing = [];
  const wanted = [...sources.ffmpeg.components.filter((c) => !c.os), ...sources.others];
  for (const s of wanted) {
    if (!s.file) {
      missing.push(`${s.name}: no file recorded`);
      continue;
    }
    const target = join(dir, s.file);
    if (s.local) {
      // Source we made ourselves (a Subversion revision with no archive): kept in this repository.
      const from = join(root, s.local);
      if (!existsSync(from) || sha256(readFileSync(from)) !== s.sha256) missing.push(`${s.name}: ${s.local} is missing or is not the recorded file`);
      else copyFileSync(from, target);
      continue;
    }
    if (!s.url || !s.sha256) {
      missing.push(`${s.name}: no URL and SHA-256 recorded`);
      continue;
    }
    if (existsSync(target) && sha256(readFileSync(target)) === s.sha256) continue;
    // A copy fetched earlier by btbn-fetch.mjs (same file name, same SHA-256) saves the download.
    const cached = CACHE_DIRS.map((d) => join(root, d, s.file)).find((f) => existsSync(f) && sha256(readFileSync(f)) === s.sha256);
    if (cached) {
      copyFileSync(cached, target);
      continue;
    }
    let got;
    try {
      got = await download(s.url, { fetchFn, retryDelayMs });
    } catch (e) {
      missing.push(`${s.name}: ${s.url} could not be fetched (${e?.message ?? e}${e?.cause ? `: ${e.cause.message ?? e.cause}` : ""})`);
      continue;
    }
    if (got.status) {
      missing.push(`${s.name}: ${s.url} answered ${got.status}`);
      continue;
    }
    const body = got.body;
    if (sha256(body) !== s.sha256) {
      missing.push(`${s.name}: ${s.url} is not the recorded file (SHA-256 differs)`);
      continue;
    }
    writeFileSync(`${target}.part`, body);
    renameSync(`${target}.part`, target);
  }
  return missing;
}

export function clearSourceBundle(dir) {
  rmSync(dir, { recursive: true, force: true });
}
