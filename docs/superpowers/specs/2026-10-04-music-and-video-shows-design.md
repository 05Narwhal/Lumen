# Music that fits, and videos in slideshows — design (Round 6)

Julian's list of 2026-10-04, written while another session works on Phase C/8f:

1. The automatic music choice feels random — make it judge the music.
2. Videos in slideshows, with ML choosing the part of each video that is used.
3. Choose the *part* of a track that fits best, and use unique Jamendo music rather than tracks
   that were already downloaded.

## Answers (2026-10-04)

| Question | Answer |
|---|---|
| How to judge music | **Listen to it**: download the shortlisted candidates and measure them with FFmpeg + the existing beat analysis. No new model. |
| Which part of a song | **The best-fitting section**: a window as long as the show, starting on a section/phrase boundary, containing the strongest section, ending on a boundary or fading. |
| Unique music | **Never pick a track that another slideshow or memory film already uses.** Saved but unused tracks remain eligible. |
| Videos | **Auto-included** in memories and new shows, **addable by hand** in the editor, **best part by ML**. Clips play **muted** under the music. |

## Why it feels random today

`music::suits` scores mood as a yes/no tag match against the wanted moods. Every result of a
`calm` search carries `calm`, durations are filtered to 60–140 % of the show, so most candidates
tie at the top and `jitter` (≤ 0.001) decides. The first query is `popularity_total`, so the same
popular tracks come back for every show, and an already-saved one is reused.

## 1. Music chosen by listening

* **Candidates.** `queries_for` drops the popularity order (all `relevance`) and gains an
  `offset` (`seed`-derived, 0/50/100) so different shows see different result pages. `MusicQuery`
  gets `offset: Option<u32>` → `&offset=`.
* **In use.** `photos_core::slideshow::music_in_use(conn)` returns the file names of every track
  referenced by `slideshows.document_json` or `memory_films` documents. A candidate whose
  `safe_id` names one of these is skipped, unless it is the track the show being rebuilt already
  had (a memory film rebuilt keeps its own tune).
* **Shortlist.** Tag-score every usable, not-in-use result (`suits_found`, tags weighted over
  *all* wanted moods, not the max), keep the best `SHORTLIST` = 6.
* **Listen.** Each shortlisted track's audio is fetched (`Searcher::fetch`) and profiled
  (`photos_core::music_profile::profile`, from mono PCM decoded by FFmpeg): tempo (beat tracker),
  energy (onsets/s), brightness (spectral centroid, normalised), loudness (RMS dB), dynamics
  (spread of the loudness curve), a 0.5 s loudness curve and its sections. Fetches run at most 3 at
  once; a failed fetch/analysis drops that candidate only.
* **Match.** Each Jamendo mood has a target `SoundTarget { tempo, energy, brightness, dynamics }`
  (`music::MOOD_SOUND`), blended by the photos' mood weights (+ the photos' energy when known).
  `sound_fit = 1 − weighted distance` in 0..1. Final score
  `0.5·sound_fit + 0.3·tag_fit + 0.2·length_fit − dramatic − recent`. Without FFmpeg the
  shortlist is ranked on tags alone (today's behaviour, minus the ties).
* **Saving.** Only the winner is written to `<library>/music` (the bytes already in hand), and
  its profile is cached in `track_profiles`. The saved-library fallback (offline) excludes in-use
  tracks and uses cached profiles where present.

## 2. The part of the song

`music_profile::choose_window(profile, beats, downbeats, target_ms) -> MusicWindow { start_ms,
fade_in_ms, fade_out_ms }`:

* **Sections**: loudness/brightness feature curve at 0.5 s, novelty = difference of the mean of
  the 4 s before vs after each point; peaks above the median become boundaries, snapped to the
  nearest downbeat (beat if no downbeats). Each section has a mean loudness.
* **Candidates**: every boundary (and 0) as a start such that the window fits, skipping starts in
  near-silence (< −35 dBFS).
* **Score**: contains the loudest section (+), mean loudness of the window (+), the window's end
  within 2 s of a boundary or the track's end (+, then fade 1 s, else fade 2.5 s), starting at 0
  when the intro is already representative (small bonus, so a song that works from the top plays
  from the top).
* A track shorter than the show starts at 0 (padded with silence as today).

The window is applied in `build_for_memory` and by a new `fit_slideshow_music(id)` command the
editor calls after *Use* in the picker. `slideshow::snap_to_beat_grid` already trims to a beat;
it now trims to the first beat **at or after** `start_ms` rather than the track's first beat.

## 3. Videos in slideshows

### Document

`Clip.video: Option<VideoSpan { in_ms, out_ms }>` (`#[serde(default, skip_serializing_if =
"Option::is_none")]`, so existing documents and golden files are unchanged). A video clip is
`fill` or `blur` only, `extra` empty, motion `none`, and its `duration_ms` defaults to the span
(clamped to the clip limits). `normalized()` enforces this. `get()` keeps videos (`media_type =
1`) whose clip has a span.

### Finding the best part

* `photos_media::video_frames::sample(ff, path, every_ms, width)` pipes `rawvideo rgb24` at 2 fps,
  192 px wide (tone-mapped when HDR, falling back like `poster_frame_with`), and returns per-sample
  RGB plus `FrameStats { t_ms, sharpness (Laplacian variance), luma, clipped, motion (mean abs
  diff vs previous), cut (histogram distance vs previous) }`. Audio loudness per 0.5 s comes from
  `beats::decode_mono` at 8 kHz; no audio track is not an error.
* In the binary, `video_highlights.rs` adds per-sample ML where the models are ready and smart
  features are on: face count (YuNet on the sample) and CLIP aesthetic on every second sample.
* `photos_core::video_highlight::best_window(samples, duration_ms, trim, min_ms, max_ms)`:
  per-sample score `0.30·sharp_rank + 0.20·exposure + 0.20·aesthetic + 0.15·faces + 0.10·motion_fit
  + 0.05·loudness` (missing ML terms re-normalise the weights). `motion_fit` rewards moderate motion
  and penalises shake. Cuts (histogram distance > 0.5) are hard walls. The window (3–6 s,
  `HIGHLIGHT_MIN_MS`/`HIGHLIGHT_MAX_MS`) maximises mean score, ties to the longer, never crosses a
  cut, stays inside the video's trim segments when it has an edit, and avoids the first/last
  0.3 s. Videos shorter than 3 s use the whole video.
* Cached in `video_highlights (asset_id PK, analyzer, in_ms, out_ms, score, computed_at)` —
  migration **0028** (0027 is left for the other session). An analyser change recomputes.
* A background job (`video-highlights`, maintenance lane, below normal priority) works through
  videos without a current highlight after import and at startup. *Add video* in the editor
  computes on demand.

### Using them

* `orderables_for` reads the highlight; an `Orderable` with `video: Some(span)` scores
  `0.5 + 0.5·highlight score` and is always grouped alone (`group_moment`). `build_arranged`
  emits it as a `fill` (landscape) or `blur` (portrait) clip with the span.
* Memories: `curated_ids` already includes videos; slideshow building now keeps the ones with a
  highlight and drops the rest, so a memory never waits on analysis.
* `create_slideshow` with hand-picked ids computes missing highlights first (FFmpeg absent: the
  video's first `HIGHLIGHT_MAX_MS`).

### Preview and export

* Preview (`slideshow-preview`): a video clip's source is an `HTMLVideoElement`
  (`photos://video/{id}`, or `?proxy=1` when `video_playback` says so), muted, drawn into the
  canvas like an image. While its clip is on screen it plays at `in_ms + (t − start)`; a drift of
  over 150 ms seeks; off screen it pauses.
* Export: each video clip is rendered first to `clip{i}.mp4` at the output size and fps
  (`photos_media::slideshow::video_clip_args`: `-ss in -t dur`, tone map when HDR, fill = cover
  crop, blur = blurred cover + fitted copy, `-an`, x264 ultrafast crf 14). `clip_filter` takes a
  video clip as `trim=start_frame=skip:end_frame=frames,setpts=PTS-STARTPTS,tpad=stop_mode=clone`
  rather than `zoompan`, so pieces and the single run treat it identically.

## Out of scope

The video's own sound (muted by answer), CLAP or any new model, editing a track's window by
dragging on a waveform (the window is chosen; *Fit to music* re-chooses it).

## Testing

TDD in Rust and TS: profile features on synthesised audio (quiet intro → loud chorus; a bright vs
dark tone; slow vs fast clicks), the window choosing the chorus and boundaries, sound matching
ranking a calm slow track above a fast bright one for *calm*, in-use exclusion, query offsets,
highlight windows on synthetic stats (best run chosen, never across a cut, inside a trim, short
video whole), normalisation of video clips, `ffmpeg_args` for a video clip, the preview drawing a
video source. `--ignored` live checks: an export with a real video clip through FFmpeg; the video
sampler on a generated test clip.
