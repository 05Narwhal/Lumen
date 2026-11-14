import assert from "node:assert/strict";
import { test } from "node:test";
import { gplCompatible } from "./compat.mjs";

test("passes permissive and GPL-compatible licences", () => {
  for (const id of ["MIT", "MIT OR Apache-2.0", "Apache-2.0 WITH LLVM-exception", "LGPL-2.1", "MPL-2.0", "GPL-3.0-or-later", "MIT/Apache-2.0", "(MIT OR CC0-1.0) AND Unicode-3.0"]) {
    assert.equal(gplCompatible(id).ok, true, id);
  }
});

test("an OR passes with one good side, an AND needs every side", () => {
  assert.equal(gplCompatible("EPL-1.0 OR MIT").ok, true);
  assert.equal(gplCompatible("MIT AND EPL-1.0").ok, false);
});

test("refuses incompatible licences and names the id", () => {
  for (const id of ["GPL-2.0-only", "EPL-1.0", "CC-BY-NC-4.0", "SSPL-1.0"]) {
    const r = gplCompatible(id);
    assert.equal(r.ok, false, id);
    assert.ok(r.reason.includes(id), `${id}: ${r.reason}`);
  }
});

test("an unknown id fails with a reason", () => {
  const r = gplCompatible("Foo-1.0");
  assert.equal(r.ok, false);
  assert.equal(r.reason, "unknown licence Foo-1.0");
});

test("an empty expression fails", () => {
  assert.equal(gplCompatible("").ok, false);
  assert.equal(gplCompatible(undefined).ok, false);
});
