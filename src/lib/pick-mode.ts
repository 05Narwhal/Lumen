// Round 19 §5: *Add photos* on an album opens the Library in pick mode, `/library?pick=album:<id>`.

/** The album a URL's query string is adding photos to; `null` when it is not pick mode. */
export function pickAlbumId(search: string): number | null {
  const m = /^album:(\d+)$/.exec(new URLSearchParams(search).get("pick") ?? "");
  return m ? Number(m[1]) : null;
}

/** Where *Add photos* on this album goes. */
export const pickAlbumUrl = (albumId: number) => `/library?pick=album:${albumId}`;

/** True while the window shows the Library in pick mode: the app-wide keys that act on the selection stand down. */
export const picking = () => pickAlbumId(window.location.search) !== null;

/**
 * Round 20 §6: whether the left-most sidebar shows as its rail. The saved setting always wins;
 * while *Add photos* shows the sidebar rails by itself unless the person has peeked at it.
 */
export const sidebarRail = (setting: boolean, picking: boolean, peek: boolean) => setting || (picking && !peek);
