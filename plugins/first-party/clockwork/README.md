# Clockwork for Lumen

Finds a camera whose clock was off and proposes corrected dates for its photos. A camera set to the
wrong time zone, or a few hours (or years) out after a battery change, files its photos on the wrong day
and in the wrong order next to the ones from your phone. Clockwork lines the two up.

## How it decides

1. It picks the cameras whose clock it can **trust**: a camera whose photos record their own UTC offset (a
   phone) or whose own GPS fixed the time. It never changes those.
2. For every other camera it looks at each **trip** on its own (photos less than two days apart) and tries
   every shift from -15 h to +15 h: how many of the camera's moments have a trusted photo within three
   minutes after the shift?
3. A real offset is a sharp spike far above what chance gives at the shifts around it, with no rival spike
   and a shift of two minutes or more. Only then does it propose a date for each photo of that trip, moved
   by that shift. A shift within two minutes of a quarter hour is rounded to it (a time zone).
4. Every proposal carries a confidence and a sentence such as *Nikon Z6's clock looks 2 h 14 min ahead of
   Pixel 8 on these days: 49 of its 60 moments line up once shifted.*

You review each date on Lumen's date review page (*Accept*, *Edit*, *Reject*, or *Accept all* above a
confidence you choose), with a 6 s undo. Accepting changes only Lumen's record of the capture time, never
the file.

## What it needs, and what it leaves alone

- At least one trusted camera used at the **same events** as the camera it checks. With only a phone or
  only a camera there is nothing to compare, and it says so.
- A few shots a trip that were taken **at the same moments** (within minutes). Two photographers who
  covered the same day but never shot together look like chance, and are left alone.
- A trip where the camera is right, or where the evidence is weak or splits between two shifts, gets no
  proposal. A trip with nothing to compare is never given another trip's shift.
- Photos whose date came from the file itself or from an edit are not clock readings and are ignored.
- The shift is found to within a minute or two. It is a correction of the clock, not of the seconds.

The setting *Propose a date only above* (default 0.7) raises or lowers how sure it has to be.

## Permissions

`library.read` (to read dates, cameras and where each date came from, never a photo's bytes) and
`dates.write` (to propose dates for the review). It has no network, files or processes.

The screenshot is a **drawing** with made-up photos, not a picture of Lumen. MIT.
