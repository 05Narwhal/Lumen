import { MotionGlobalConfig } from "motion/react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { useSeqStore } from "@/hooks/use-tag-search";
import { describe as words } from "@/lib/search-expr";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagSeq } from "@/types/generated/TagSeq";
import { TokenRow } from "./token-row";

MotionGlobalConfig.skipAnimations = true;

const kind = (label: string, k: string): SearchTag => ({ label, value: { type: "kind", kind: k } as SearchTag["value"] });
const tag = (t: SearchTag) => ({ type: "tag" as const, tag: t });

/** A row over a local store, with the sequence in words beside it. */
function Harness({ initial }: { initial: TagSeq }) {
  const store = useSeqStore(initial);
  return (
    <>
      <div role="list">
        <TokenRow seq={store.seq} ops={store} />
      </div>
      <output data-testid="words">{words(store.seq)}</output>
    </>
  );
}

const mount = (initial: TagSeq) =>
  render(
    <Providers>
      <Harness initial={initial} />
    </Providers>,
  );

describe("TokenRow", () => {
  it("a join menu turns or into and", async () => {
    const user = userEvent.setup();
    mount({ items: [tag(kind("Live Photos", "live")), tag(kind("HDR", "hdr"))], joins: ["or"] });
    expect(screen.getByTestId("words").textContent).toBe("Live Photos or HDR");
    await user.click(screen.getByRole("button", { name: "or — change how these combine" }));
    await user.click(await screen.findByRole("menuitem", { name: "and" }));
    expect(screen.getByTestId("words").textContent).toBe("Live Photos and HDR");
    expect(screen.getByRole("button", { name: "and — change how these combine" })).toBeTruthy();
  });

  it("Group these two draws a pill, and Ungroup takes it apart", async () => {
    const user = userEvent.setup();
    mount({ items: [tag(kind("2026", "year")), tag(kind("Live Photos", "live")), tag(kind("HDR", "hdr"))], joins: ["and", "or"] });
    expect(screen.queryByRole("group", { name: "Grouped tags" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "or — change how these combine" }));
    await user.click(await screen.findByRole("menuitem", { name: "Group these two" }));
    expect(screen.getByRole("group", { name: "Grouped tags" })).toBeTruthy();
    expect(screen.getByTestId("words").textContent).toBe("2026 and (Live Photos or HDR)");
    await user.click(screen.getByRole("button", { name: "or — change how these combine" }));
    await user.click(await screen.findByRole("menuitem", { name: "Ungroup" }));
    expect(screen.queryByRole("group", { name: "Grouped tags" })).toBeNull();
  });
});
