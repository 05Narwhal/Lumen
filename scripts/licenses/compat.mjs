// Can a dependency under this licence be part of a GPL-3.0-or-later program?
// The same AND / OR rules as photos-plugins' licence parser: an OR passes if one side passes,
// an AND only if every side does. `yarn run licenses` refuses a build whose shipped dependencies
// fail, so an incompatible licence can't slip in with an update.

const ALLOW = new Set([
  "MIT", "MIT-0", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense",
  "MPL-2.0", "CC0-1.0", "CC-BY-4.0", "OFL-1.1", "Unicode-3.0", "Unicode-DFS-2016", "BSL-1.0",
  "LGPL-2.1", "LGPL-2.1-only", "LGPL-2.1-or-later", "LGPL-3.0", "LGPL-3.0-only", "LGPL-3.0-or-later",
  "GPL-2.0-or-later", "GPL-3.0", "GPL-3.0-only", "GPL-3.0-or-later",
  "ODbL-1.0", // data only
  "CDLA-Permissive-2.0", // webpki-roots, the CA bundle data: permissive, keep the licence text with it
  "IJG", // jpeg-encoder: BSD-style, credit only, GPL-compatible
]);

/** Known to clash with the GPL-3 (or to forbid commercial or hosted use); the reason says why. */
const DENY = new Map([
  ["GPL-2.0", "GPL-2.0-only can't be combined with GPL-3.0"],
  ["GPL-2.0-only", "GPL-2.0-only can't be combined with GPL-3.0"],
  ["EPL-1.0", "EPL-1.0 is incompatible with the GPL"],
  ["EPL-2.0", "EPL-2.0 is incompatible with the GPL unless it names a secondary licence"],
  ["SSPL-1.0", "SSPL-1.0 is not a free-software licence"],
  ["BUSL-1.1", "BUSL-1.1 is not a free-software licence"],
  ["CDDL-1.0", "CDDL-1.0 is incompatible with the GPL"],
]);

function tokens(expr) {
  return String(expr)
    .replace(/\//g, " OR ") // Cargo writes `MIT/Apache-2.0`
    .replace(/([()])/g, " $1 ")
    .split(/\s+/)
    .filter(Boolean);
}

function parse(toks) {
  let at = 0;
  const peek = () => toks[at];
  const atom = () => {
    const t = toks[at++];
    if (t === undefined) throw new Error("the expression ends too early");
    if (t === "(") {
      const e = or();
      if (toks[at++] !== ")") throw new Error("missing ')'");
      return e;
    }
    if ([")", "AND", "OR", "WITH"].includes(t)) throw new Error(`unexpected ${t}`);
    if (peek() === "WITH") {
      at++;
      if (toks[at++] === undefined) throw new Error("WITH needs an exception");
    }
    return { id: t };
  };
  const and = () => {
    let l = atom();
    while (peek() === "AND") {
      at++;
      l = { and: [l, atom()] };
    }
    return l;
  };
  const or = () => {
    let l = and();
    while (peek() === "OR") {
      at++;
      l = { or: [l, and()] };
    }
    return l;
  };
  const e = or();
  if (at < toks.length) throw new Error(`unexpected ${toks[at]}`);
  return e;
}

function judge(e) {
  if (e.id !== undefined) {
    if (ALLOW.has(e.id)) return { ok: true };
    return { ok: false, reason: DENY.has(e.id) ? `${e.id}: ${DENY.get(e.id)}` : /-NC|-ND/.test(e.id) ? `${e.id} forbids commercial use or changes` : `unknown licence ${e.id}` };
  }
  if (e.and) {
    const rs = e.and.map(judge);
    return rs.find((r) => !r.ok) ?? { ok: true };
  }
  const rs = e.or.map(judge);
  return rs.find((r) => r.ok) ?? { ok: false, reason: rs.map((r) => r.reason).join("; ") };
}

/** `{ ok: true }`, or `{ ok: false, reason }` naming the id that failed. */
export function gplCompatible(spdx) {
  const toks = tokens(spdx ?? "");
  if (!toks.length) return { ok: false, reason: "no licence is given" };
  try {
    return judge(parse(toks));
  } catch (e) {
    return { ok: false, reason: `unreadable licence "${spdx}": ${e.message}` };
  }
}
