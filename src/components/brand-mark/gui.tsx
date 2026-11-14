import large from "@/assets/brand/lumen-512.png";
import small from "@/assets/brand/lumen-64.png";
import "./styles.scss";

/**
 * Lumen's logo (Round 8B), the same picture as the app icon. Decorative: it always sits beside
 * the name. Up to 32 px it is the 64 px file (sharp at 2×), above that the 512 px one.
 */
export function BrandMark({ size }: { size: number }) {
  return <img className="brand-mark" src={size <= 32 ? small : large} width={size} height={size} alt="" draggable={false} />;
}
