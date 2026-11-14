import type { CSSProperties, ReactNode } from "react";
import type { CoverShape } from "@/types/generated/CoverShape";
import type { CoverStyle } from "@/types/generated/CoverStyle";
import { coverColor, iconNamed } from "./icons";
import "./styles.scss";

const FILL = "var(--cover-color)";
const SHADE = "var(--cover-shade)";
const LIGHT = "var(--cover-light)";

/** Each shape in a 120 × 90 box (the card's 4:3 cover). Its icon sits where `data-shape` says. */
const SHAPES: Record<CoverShape, ReactNode> = {
  folder: (
    <>
      <path d="M22 24a4 4 0 0 1 4-4h20l6 6h42a4 4 0 0 1 4 4v40a4 4 0 0 1-4 4H26a4 4 0 0 1-4-4z" fill={FILL} />
      <path d="M22 24a4 4 0 0 1 4-4h20l6 6h42a4 4 0 0 1 4 4v40a4 4 0 0 1-4 4H26a4 4 0 0 1-4-4z" fill={SHADE} />
      <path d="M22 36a4 4 0 0 1 4-4h68a4 4 0 0 1 4 4v34a4 4 0 0 1-4 4H26a4 4 0 0 1-4-4z" fill={FILL} />
      <path d="M26 32h68a4 4 0 0 1 4 4v2H22v-2a4 4 0 0 1 4-4z" fill={LIGHT} />
    </>
  ),
  book: (
    <>
      <rect x="38" y="15" width="50" height="66" rx="3" fill="var(--print-paper)" />
      <rect x="32" y="11" width="52" height="66" rx="4" fill={FILL} />
      <rect x="32" y="11" width="10" height="66" rx="3" fill={SHADE} />
      <rect x="42" y="11" width="2" height="66" fill={LIGHT} />
    </>
  ),
  box: (
    <>
      <rect x="24" y="30" width="72" height="46" rx="3" fill={FILL} />
      <rect x="24" y="30" width="72" height="7" fill={SHADE} />
      <path d="M21 31a3 3 0 0 1 0-1l5-11a3 3 0 0 1 3-2h62a3 3 0 0 1 3 2l5 11a3 3 0 0 1 0 1z" fill={FILL} />
      <path d="M21 31a3 3 0 0 1 0-1l5-11a3 3 0 0 1 3-2h62a3 3 0 0 1 3 2l5 11a3 3 0 0 1 0 1z" fill={LIGHT} />
    </>
  ),
  binder: (
    <>
      <rect x="31" y="10" width="60" height="70" rx="5" fill={FILL} />
      <rect x="31" y="10" width="13" height="70" rx="4" fill={SHADE} />
      <circle cx="37.5" cy="25" r="3" fill={LIGHT} />
      <circle cx="37.5" cy="45" r="3" fill={LIGHT} />
      <circle cx="37.5" cy="65" r="3" fill={LIGHT} />
      <rect x="52" y="24" width="32" height="42" rx="3" fill={LIGHT} />
    </>
  ),
};

/**
 * An album or collection shown as a shape with an icon on it instead of its photos (Julian's
 * 2026-11-08 list, item 3). The colour is the person's: a swatch token or their own hex, set
 * as `--cover-color` on the element. Fills its parent; lifts a little on hover like the prints.
 */
export function IconCover({ style }: { style: CoverStyle }) {
  const Icon = iconNamed(style.icon);
  return (
    <span className="icon-cover" data-shape={style.shape} style={{ "--cover-color": coverColor(style.color) } as CSSProperties} aria-hidden>
      <svg className="icon-cover__shape" viewBox="0 0 120 90" preserveAspectRatio="xMidYMid meet">
        {SHAPES[style.shape] ?? SHAPES.folder}
      </svg>
      <span className="icon-cover__icon">
        <Icon stroke={1.75} />
      </span>
    </span>
  );
}
