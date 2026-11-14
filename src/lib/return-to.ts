/**
 * The last page outside Settings (Round 6), so Settings' Back returns to it. The root layout
 * records every location; Settings itself is never recorded.
 */
let last: string | null = null;

export function rememberPage(path: string | null) {
  last = path;
}

/** Where Back goes: the page Settings was opened from, or Home. */
export const returnPath = () => last ?? "/";
