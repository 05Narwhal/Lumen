import { IconCover } from "@/components/icon-cover";
import type { CoverStyle } from "@/types/generated/CoverStyle";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** Where each print lies: the cover in front, the others tucked behind to the left and right. */
const POSITIONS = ["front", "left", "right"] as const;

/** One print in a stack: a photo, or an album's or collection's icon cover (in a collection's stack). */
export type CoverItem = { kind: "photo"; id: number } | { kind: "icon"; style: CoverStyle };

/** An album's prints, front photo first. */
export const photoItems = (ids: number[]): CoverItem[] => ids.map((id) => ({ kind: "photo", id }));

const keyOf = (item: CoverItem, i: number) => (item.kind === "photo" ? `p${item.id}` : `i${i}`);

/**
 * Up to three prints fanned like a small pile on a table (Julian's 2026-11-08 list; plain
 * rounded squares since 2026-11-09). The first item lies on top. Hovering or focusing the link
 * or button it sits in fans them out a little; reduced motion keeps them still. Fills its
 * parent, which sets the size.
 */
export function CoverStack({ items }: { items: CoverItem[] }) {
  const prints = items.slice(0, 3);
  return (
    <span className="cover-stack" data-count={prints.length} aria-hidden>
      {prints.map((item, i) => (
        <span key={keyOf(item, i)} className="cover-stack__print" data-pos={POSITIONS[i]} data-kind={item.kind}>
          {item.kind === "photo" ? (
            <img src={mediaUrl("thumb", item.id, { s: 1024 })} alt="" loading="lazy" draggable={false} />
          ) : (
            <IconCover style={item.style} />
          )}
        </span>
      ))}
    </span>
  );
}
