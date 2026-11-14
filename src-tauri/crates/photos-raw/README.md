# photos-raw

Lumen's RAW decoding: the only code in Lumen that uses [`rawler`](https://github.com/dnglab/dnglab) (LGPL-2.1). It is built alone as a shared library and loaded when Lumen runs, so you can rebuild it, change it or swap `rawler` for your own copy without Lumen's source.

## Build it

From `src-tauri/`, with no other Lumen crate involved:

```sh
cargo build -p photos-raw --profile raw-lib
```

Keep the `raw-lib` profile (or at least `panic = "unwind"`): Lumen's release build aborts on a panic, and this library's panics are what `catch_unwind` turns into a status. The result is in `target/raw-lib/`: `photos_raw.dll`, `libphotos_raw.so` or `libphotos_raw.dylib`.

## Use your own

Replace the file Lumen installed, or set `LUMEN_PHOTOS_RAW` to the absolute path of yours. Lumen looks in this order, by absolute path only: `LUMEN_PHOTOS_RAW`, its resources folder (`raw/`), beside the executable.

A library that's missing, is of another ABI version or isn't this library is a state, not a crash: RAW files keep their camera JPEG and metadata from EXIF, and Develop says *Camera rendering, not RAW*. Only panics are contained; a stack overflow or an abort inside the library ends Lumen.

## The interface

`PHOTOS_RAW_ABI` is `1`. `src/lib.rs` is the reference.

| Export | |
|---|---|
| `photos_raw_abi() -> u32` | The version; Lumen refuses any other. |
| `photos_raw_decode_linear(path, path_len, interactive, out) -> status` | Calibrated sensor data as linear sRGB, `[f32; 3]` per pixel. |
| `photos_raw_preview(path, path_len, interactive, out) -> status` | The embedded preview; `STATUS_OK` with nothing when there is none. |
| `photos_raw_metadata(path, path_len, interactive, out) -> status` | JSON: `make`, `model`, `serial`, `date_time_original`. |
| `photos_raw_free(out)` | Frees an `Output`. Call it exactly once per call, whatever the status. |
| `photos_raw_test_panic(out)`, `photos_raw_test_where(interactive, out)` | Used by Lumen's tests of this boundary. |

`path` is UTF-16 on Windows and bytes elsewhere, and `path_len` counts those units. `interactive` is `1` for work someone is waiting on and `0` for background work, which runs below normal priority. Status: `0` ok, `1` refused, `2` I/O, `3` failed, `4` panicked. With any status but `0`, `out` holds the reason as UTF-8.

## Licence and notices

This crate is LGPL-2.1, as `rawler` is. Lumen's other crates keep their own licence; Settings → *Licences* lists `rawler` and everything it brings.

`rawler` reads DNG files, so the DNG Specification patent license's notice applies: *This product includes DNG technology under license by Adobe.*
