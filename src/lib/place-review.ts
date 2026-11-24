import { createStore } from "@/lib/store";

/**
 * The Place review screen's actions as the palette reaches them (Round 24 W4). The screen
 * sets `open` while it is mounted, so the entries are offered only there; a pick is a
 * request the screen answers, the way Develop's Auto and Retouch buttons are reached.
 */
export type PlaceReviewAction = "confirm" | "reject" | "move-place" | "move-pin" | "split" | "write" | "next" | "previous";

export const placeReviewOpen = createStore(false);
export const placeReviewRequests = createStore<{ n: number; action: PlaceReviewAction | null }>({ n: 0, action: null });

export const requestPlaceReview = (action: PlaceReviewAction) => placeReviewRequests.set((s) => ({ n: s.n + 1, action }));
