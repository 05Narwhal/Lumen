import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { makeSourceBundle } from "./source-bundle.mjs";

const BODY = Buffer.from("source bytes");
const sha = createHash("sha256").update(BODY).digest("hex");
const sources = {
  ffmpeg: { components: [{ name: "zlib", flag: "zlib", url: "https://example/zlib.tar.gz", sha256: sha, file: "zlib.tar.gz" }] },
  others: [],
};
const answer = () => new Response(BODY, { status: 200 });
const dropped = () => Promise.reject(new TypeError("terminated"));

let dir;
afterEach(() => dir && rmSync(dir, { recursive: true, force: true }));
const fresh = () => (dir = mkdtempSync(join(tmpdir(), "lumen-src-")));

describe("the recorded sources", () => {
  it("list no photos-raw archive: rawler's source is the tagged public repository", () => {
    const recorded = JSON.parse(readFileSync(join(process.cwd(), "scripts/release/sources.json"), "utf8"));
    expect(recorded.others.filter((o) => o.repo || /photos-raw/.test(o.name))).toEqual([]);
  });
});

describe("makeSourceBundle", () => {
  it("retries a dropped connection and still gets the file", async () => {
    let calls = 0;
    const fetchFn = () => (++calls < 3 ? dropped() : Promise.resolve(answer()));
    const missing = await makeSourceBundle("/nowhere", sources, fresh(), { fetchFn, retryDelayMs: 0 });
    expect(missing).toEqual([]);
    expect(calls).toBe(3);
    expect(readFileSync(join(dir, "zlib.tar.gz"))).toEqual(BODY);
  });

  it("reports a connection that never holds instead of ending the release script", async () => {
    const missing = await makeSourceBundle("/nowhere", sources, fresh(), { fetchFn: dropped, retryDelayMs: 0 });
    expect(missing).toHaveLength(1);
    expect(missing[0]).toMatch(/zlib.*terminated/);
    expect(existsSync(join(dir, "zlib.tar.gz"))).toBe(false);
    expect(existsSync(join(dir, "zlib.tar.gz.part"))).toBe(false);
  });

  it("does not retry an answer that is a plain no", async () => {
    let calls = 0;
    const fetchFn = () => (++calls, Promise.resolve(new Response("", { status: 404 })));
    const missing = await makeSourceBundle("/nowhere", sources, fresh(), { fetchFn, retryDelayMs: 0 });
    expect(missing[0]).toMatch(/answered 404/);
    expect(calls).toBe(1);
  });
});
