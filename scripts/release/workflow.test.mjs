// What .github/workflows/release.yml must keep promising to the security track (release
// simplification phase 4). There is no YAML parser in the project, so these read the text.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const workflow = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "..", ".github/workflows/release.yml"), "utf8").replace(/\r\n/g, "\n");

/** The text of one top-level job. */
const job = (name) => {
  const start = workflow.indexOf(`\n  ${name}:\n`);
  if (start < 0) throw new Error(`no job ${name}`);
  const rest = workflow.slice(start + 1);
  const next = rest.slice(1).search(/\n  [a-z-]+:\n/);
  return next < 0 ? rest : rest.slice(0, next + 1);
};

describe("release.yml", () => {
  it("runs for a v* tag whatever branch it is on, so a tag from release/26.X is built too (tag-ref judges the branch)", () => {
    const on = workflow.slice(workflow.indexOf("\non:\n"), workflow.indexOf("\npermissions:"));
    expect(on).toMatch(/tags:\s*\n\s*- "v\*"/);
    expect(on).not.toMatch(/branches/);
  });

  it("sees every tag and branch of the checkout, which tag-ref and no-migration read", () => {
    expect(job("preflight")).toMatch(/fetch-depth: 0/);
  });

  it("updates the channel manifests one release at a time, so a hotfix and an alpha cannot overwrite each other's read", () => {
    const publish = job("publish");
    expect(publish).toMatch(/concurrency:\s*\n\s*group: release-channels\s*\n\s*cancel-in-progress: false/);
  });
});
