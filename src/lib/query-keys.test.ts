import { describe, expect, it } from "vitest";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { queryKeys, scopeKey } from "./query-keys";

describe("scopeKey", () => {
  it("tells people, the Hidden folder and every other scope apart", () => {
    const a = scopeKey({ ...LIBRARY_SCOPE, personId: 1 });
    const b = scopeKey({ ...LIBRARY_SCOPE, personId: 2 });
    expect(a).not.toBe(b);
    expect(scopeKey({ ...LIBRARY_SCOPE, vault: true })).not.toBe(scopeKey(LIBRARY_SCOPE));
    expect(queryKeys.layout(800, 200, 4, { ...LIBRARY_SCOPE, personId: 1 })).not.toEqual(queryKeys.layout(800, 200, 4, { ...LIBRARY_SCOPE, personId: 2 }));
  });

  it("does not depend on the order fields were written in", () => {
    const x = { ...LIBRARY_SCOPE, personId: 3, albumId: null };
    const y = Object.fromEntries(Object.entries(x).reverse()) as typeof x;
    expect(scopeKey(x)).toBe(scopeKey(y));
  });
});
