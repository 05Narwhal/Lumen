# Captioner for Lumen

Suggests a **title** and a **caption** for photos that have none, from what Lumen already knows about
them: who is in the photo (people you have named), where and when it was taken, what Lumen saw in it
(*sunset*, *beach*) and text it read (a sign). There is no model and nothing leaves your computer.

> **Anna and Ben in Idrija**
> Anna and Ben in Idrija, Slovenia. Thursday 12 March 2026, in the evening. Looks like sunset and sea.
> Reads “OPEN 24h”.

You review each suggestion on Lumen's title and caption review pages (*Accept*, *Edit*, *Reject*, or
*Accept all* above a confidence you choose), with a 6 s undo. Each says what it was built from. Accepting
sets the title or caption in Lumen; no file is touched.

## Rules

- A photo that already has a title keeps it, and the same for a caption.
- A title needs a person, a place or a scene. A date alone is never a title.
- Confidence is 0.8 with a person, a place and a scene, 0.65 with two, 0.5 with one, and 0.1 less when the
  place was only estimated (Wayfinder, a folder's name).
- Text is quoted only when it reads like a sign: a short line made mostly of letters. Numbers, prices and
  pages of print are never quoted, and a photo that is mostly text gets none.
- Vague scene words (*group of people*, *selfie*) never make a title.

Settings: titles on or off, captions on or off, quote text, name the album (off by default), and the lowest
confidence to suggest (0.5). The work picks up where it stopped if a library is big enough to take several
runs.

## Permissions

`library.read` (to read people, places, dates, scene words, text and albums, never a photo's bytes),
`captions.write` (to propose titles and captions for the review) and `storage` (to remember where a run
stopped). It has no network, files or processes.

The screenshot is a **drawing** with made-up data, not a picture of Lumen. MIT.
