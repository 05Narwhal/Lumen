import { createStore } from "./store";

/**
 * *Tag people…* (the grid's menu, the palette): the photo whose viewer should open with every
 * face's box shown and the info panel out. The viewer takes the request when that photo is
 * open, and clears it.
 */
export const faceTaggingRequest = createStore<number | null>(null);

export function requestFaceTagging(assetId: number) {
  faceTaggingRequest.set(assetId);
}

/** *Show pets* from the palette (Round 8B): the photo whose viewer should toggle its pets' boxes. */
export const petBoxesRequest = createStore<number | null>(null);
