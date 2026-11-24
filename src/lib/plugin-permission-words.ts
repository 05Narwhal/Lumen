// Round 24 lane C: the store lists a plugin's permissions by their manifest names (`library.read`);
// this words them for the detail sheet. The first four are the install sheet's own sentences
// (`PERMISSION_LABELS`, spec Round 23 §3), the rest are platform 1.1's.

const WORDS: Record<string, string> = {
  "import.read-chosen": "Read the folder or file you choose when you import with it",
  "photos.read": "See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place",
  "develop.write-drafts": "Change Develop drafts of those photos (every change can be undone)",
  storage: "Keep its own files (up to 50 MB)",
  models: "Run smart-feature models on photos",
  "library.read": "Read your library's photos: when and where they were taken, their titles, captions, albums, the names of people in them and the text Lumen read in them",
  "places.write": "Suggest places for photos; nothing changes until you review and accept",
  "originals.gps": "Write a place into the original file of a photo that has none, after you confirm",
  // API 1.3 (Round 25 P4b)
  "dates.write": "Suggest dates for photos; nothing changes until you review and accept",
  "tags.write": "Suggest tags and labels for photos; nothing changes until you review and accept",
  "captions.write": "Suggest titles and captions for photos; nothing changes until you review and accept",
  "people.write": "Suggest names for faces Lumen found; nothing changes until you review and accept",
};

/** A permission in plain words; a name this Lumen doesn't know is shown as it is. */
export const permissionWords = (name: string): string => WORDS[name] ?? name;

/** What a plugin that lists no permissions can do. */
export const NO_PERMISSIONS = "It asks for no permissions.";

/** Bytes as the store shows a package's size. */
export function packageSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
