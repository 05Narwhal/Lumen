import { signed } from "@/lib/develop";

/** A ±1 value shown as Lightroom does, ×100. The slider already works in those units. */
export const percent = (v: number) => signed(Math.round(v));
export const stops = (v: number) => signed(v, 2);
export const whole = (v: number) => String(Math.round(v));
export const pixels = (v: number) => v.toFixed(1);
export const degrees = (v: number) => `${Math.round(v)}°`;
