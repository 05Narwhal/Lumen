# Wayfinder for Lumen

Guesses where photos **without a location** were taken, and lets you check the guesses before anything is
saved. Run *Find locations…* (the `find_locations` tool) on the selection, an album or the whole library;
the Place review opens with the guesses on a map, moment by moment. Nothing is written until you press
*Write N locations*, and one undo takes the whole write back.

What it uses, in order of trust:

1. **Anchors**: photos with camera GPS and places you confirmed. Never changed.
2. **Time**: a photo between two anchors within two hours is placed by interpolation (if the trip is
   possible at 250 km/h); one anchor within 15 minutes gives its place. Bursts and Live pairs share a place.
3. **Your own photos**: the most similar photo you already took with GPS (similarity 0.65 or more).
4. **GeoCLIP**: what the photo shows, against 100,000 places, weighted towards where you have been, with
   extra points around your own places.
5. **Folder names**: a folder called `Paris 2024` weighs a moment's guesses near Paris.

Guesses far from their moment's place, impossible in speed, or low in confidence go to *Check these*.

It needs the `models`, `library.read`, `places.write`, `originals.gps` and `storage` permissions. With
`originals.gps` the confirmed places are also written into the original files that have none (JPEG, HEIC,
PNG, TIFF, DNG, NEF), verified, and marked `ESTIMATED:jl.wayfinder`; videos and other RAW formats keep the
place in Lumen only. The setting *Write locations into original files* turns that off.

The models (about 750 MB, downloaded once after you agree on the install sheet) are GeoCLIP exported to
ONNX, **CC-BY-NC-4.0**: free for personal use, not for commercial use (`docs/THIRD-PARTY-MODELS.md`).

## Credits

- **FindMyPhoto** gave the idea of guessing a photo's place from its
  pixels and the library around it. It is AGPL; none of its code is used.
- **GeoCLIP** (Vivanco, Nayak, Jha and Shah, NeurIPS 2023; code and weights MIT) is the model.
- The plugin's code is MIT.

## Building

```sh
cd plugins/first-party/wayfinder
cargo test                                  # the logic, natively
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/wayfinder.wasm plugin.wasm
node ../../../scripts/plugins/pack.mjs pack .
```
