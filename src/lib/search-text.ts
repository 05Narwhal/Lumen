// Editing a search's text through its chips (Round 4K). Rust reports each chip's span in
// UTF-8 bytes; JavaScript strings count UTF-16 units, so "Zürich" needs converting.

const encoder = new TextEncoder();

/** The string index of UTF-8 byte offset `byte` in `text`. */
export function byteToIndex(text: string, byte: number): number {
  let bytes = 0;
  let i = 0;
  while (i < text.length && bytes < byte) {
    const cp = text.codePointAt(i) ?? 0;
    bytes += encoder.encode(String.fromCodePoint(cp)).length;
    i += cp > 0xffff ? 2 : 1;
  }
  return i;
}

function tidy(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** `text` without the part a chip read (removing the chip). */
export function removeSpan(text: string, span: [number, number]): string {
  const [a, b] = [byteToIndex(text, span[0]), byteToIndex(text, span[1])];
  return tidy(text.slice(0, a) + " " + text.slice(b));
}

/** `text` with the part a chip read written as `token` (choosing another reading). */
export function replaceSpan(text: string, span: [number, number], token: string): string {
  const [a, b] = [byteToIndex(text, span[0]), byteToIndex(text, span[1])];
  return tidy(text.slice(0, a) + token + text.slice(b));
}
