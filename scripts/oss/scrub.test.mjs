import { test } from "node:test";
import assert from "node:assert/strict";
import { scanText, scanFile, loadRules, selectFiles } from "./scrub.mjs";
import { checkDest } from "./export-public.mjs";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const rules = loadRules(join(here, "scrub-rules.json"));

// A JPEG whose APP1 holds a TIFF with one IFD0 entry: a pointer to a GPS IFD.
function jpegWithGps() {
  const tiff = Buffer.alloc(8 + 2 + 12 + 4 + 2 + 12 + 4);
  tiff.write("II", 0, "latin1");
  tiff.writeUInt16LE(42, 2);
  tiff.writeUInt32LE(8, 4);
  tiff.writeUInt16LE(1, 8);
  tiff.writeUInt16LE(0x8825, 10);
  tiff.writeUInt16LE(4, 12);
  tiff.writeUInt32LE(1, 14);
  tiff.writeUInt32LE(26, 18);
  tiff.writeUInt32LE(0, 22);
  tiff.writeUInt16LE(1, 26);
  tiff.writeUInt16LE(0, 28);
  tiff.writeUInt16LE(1, 30);
  tiff.writeUInt32LE(4, 32);
  tiff.writeUInt32LE(0x00000202, 36);
  tiff.writeUInt32LE(0, 40);
  const app1 = Buffer.concat([Buffer.from("Exif\u0000\u0000", "latin1"), tiff]);
  const len = Buffer.alloc(2);
  len.writeUInt16BE(app1.length + 2);
  return Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe1]), len, app1, Buffer.from([0xff, 0xd9])]);
}

test("a Windows home path is one finding", () => {
  const f = scanText("a.md", "see C:" + "\\Users\\Alice\\Pictures here", rules);
  assert.equal(f.length, 1);
  assert.equal(f[0].line, 1);
  assert.equal(f[0].rule, `deny:${rules.deny[0]}`);
});

test("emails are findings except the allowed ones", () => {
  assert.equal(scanText("a.md", "alice@" + "corp.test", rules).length, 1);
  assert.equal(scanText("a.md", "noreply@anthropic.com", rules).length, 0);
  assert.equal(scanText("a.md", "1+x@users.noreply.github.com", rules).length, 0);
});

test("a local deny entry is a plain string, matched case-insensitively", () => {
  const r = { ...rules, localDeny: ["Some.Name (x)"] };
  assert.equal(scanText("a.md", "by some.name (X)", r).length, 1);
  assert.equal(scanText("a.md", "by some-name", r).length, 0);
});

test("words rules apply only when asked for", () => {
  assert.equal(scanText("a.md", "Lumen Pro", rules).length, 0);
  const r = { ...rules, activeWords: ["pro-cloud"] };
  assert.equal(scanText("a.md", "Lumen Pro", r).length, 1);
});

test("a .pyc is refused", () => {
  const f = scanFile("scripts/x/__pycache__/a.pyc", Buffer.from([1, 2, 3]), rules);
  assert.equal(f[0].rule, "pycache");
});

test("an image outside mediaAllow is refused", () => {
  const f = scanFile("docs/shot.png", Buffer.from([0x89, 0x50, 0x4e, 0x47]), rules);
  assert.equal(f[0].rule, "media-not-allowed");
});

test("an allowed JPEG with EXIF GPS is reported", () => {
  const f = scanFile("src/assets/x.jpg", jpegWithGps(), rules);
  assert.deepEqual(f.map((x) => x.rule), ["exif:GPS"]);
});

test("an allowed JPEG without EXIF is clean", () => {
  const f = scanFile("src/assets/x.jpg", Buffer.from([0xff, 0xd8, 0xff, 0xd9]), rules);
  assert.deepEqual(f, []);
});

test("selectFiles drops excluded paths", () => {
  assert.deepEqual(
    selectFiles(["docs/superpowers/plans/a.md", "docs/STATE.md"], ["docs/superpowers/plans/**"]),
    ["docs/STATE.md"],
  );
  assert.deepEqual(
    selectFiles(["a/__pycache__/b.pyc", "CLAUDE.local.md", "x/CLAUDE.local.md", "scripts/oss/a.local.json", "k.md"],
      ["**/__pycache__/**", "CLAUDE.local.md", "scripts/oss/*.local.json"]),
    ["k.md"],
  );
});

// The export destination guard (Task 10): never a non-empty folder, the repo or a library.
test("checkDest refuses a non-empty folder", () => {
  const d = mkdtempSync(join(tmpdir(), "lumen-dest-"));
  try {
    writeFileSync(join(d, "x.txt"), "x");
    assert.match(checkDest(d, join(here, "..", ".."), []), /not empty/i);
  } finally {
    rmSync(d, { recursive: true, force: true });
  }
});

test("checkDest refuses a folder inside the repo, and the repo itself", () => {
  const repo = mkdtempSync(join(tmpdir(), "lumen-repo-"));
  try {
    assert.match(checkDest(join(repo, "out"), repo, []), /inside the repo/i);
    assert.match(checkDest(repo, repo, []), /inside the repo/i);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
});

test("checkDest refuses a folder inside, or holding, a library", () => {
  const lib = mkdtempSync(join(tmpdir(), "lumen-lib-"));
  try {
    mkdirSync(join(lib, "exports"));
    assert.match(checkDest(join(lib, "exports", "new"), "/elsewhere/repo", [lib]), /library/i);
    assert.match(checkDest(lib, "/elsewhere/repo", [lib]), /library/i);
    assert.match(checkDest(tmpdir(), "/elsewhere/repo", [lib]), /library/i);
  } finally {
    rmSync(lib, { recursive: true, force: true });
  }
});

test("checkDest accepts a new empty folder in the temp directory", () => {
  const d = mkdtempSync(join(tmpdir(), "lumen-dest-"));
  try {
    assert.equal(checkDest(d, "/elsewhere/repo", ["/elsewhere/library"]), null);
    assert.equal(checkDest(join(d, "not-yet"), "/elsewhere/repo", []), null);
  } finally {
    rmSync(d, { recursive: true, force: true });
  }
});
