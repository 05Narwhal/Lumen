// What Lumen imports, by extension, for the file picker's filter and for telling dropped
// files from dropped folders. Mirrors photos-media's `kind.rs` (IMAGE, RAW, HEIC, VIDEO);
// the backend decides for real, so a stray entry here only affects what the picker shows.
export const MEDIA_EXTENSIONS = [
  "jpg", "jpeg", "jpe", "png", "webp", "gif", "tif", "tiff", "bmp", "jxl",
  "heic", "heif", "hif", "avif",
  "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "srw", "3fr", "iiq", "erf", "mos", "mrw", "kdc", "dcr",
  "mp4", "m4v", "mov", "avi", "mkv", "mts", "m2ts", "webm", "3gp", "mpg", "mpeg", "wmv",
];

const set = new Set(MEDIA_EXTENSIONS);

export const isMediaPath = (path: string) => set.has(path.split(".").pop()?.toLowerCase() ?? "");
