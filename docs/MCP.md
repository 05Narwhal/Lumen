# AI assistants in Lumen (MCP)

An opt-in local MCP server (Settings → *AI assistants*; off by default, `127.0.0.1`, bearer key) lets Claude Code, Codex or Claude Desktop edit the photos in Develop's filmstrip. It writes **drafts and history steps only**: never the library's `edits`, never originals. Build notes live in `docs/STATE.md` (*AI assistants (MCP)*); this file is what an assistant, and a person reading its log, should know.

## What the assistant is asked to do (Round 18 D1)

Make each photo look good and distinctive, not only correct. The playbook is `mcp::INSTRUCTIONS` (sent at `initialize` and as the `edit_photo` prompt; `retouch` is kept as an alias):

1. Read the photo: `view`, `analyze` (light, palette, faces, Facet's regions).
2. Pick looks. **With a brief** (`edit_photo brief:"moody film"`) it builds that one look. **Without one** it calls `suggest_looks` and builds every look offered.
3. Build each look from the clean photo (`reset` between looks): the look's patch as a start, adapted to the photo, with curve, HSL, grading and local masks. A look must show in a before/after split.
4. Check (`view compare:"split"`, `view overlay`, `analyze`): faces never clip, highlights clip < 0.5 %, skin hue ~20–50°.
5. Keep each look with `draft_variant`; finish with `draft_variant op:"use"` on the top-scoring one.

## Looks and the fit score

`lumen://looks` (resource) lists the 12 hand-written looks (`mcp/looks.rs`): `natural`, `golden-film`, `teal-orange`, `cool-moody`, `low-key`, `high-key-airy`, `faded-matte`, `bw-contrast`, `bw-soft`, `vivid-pop`, `warm-portrait`, `night-neon`. Each has an `intent` and a starting `patch` shaped like `get_recipe`.

`suggest_looks {id}` scores every look 0–100 **for that photo** (`mcp/fit.rs`): 45 % colour harmony (hue histogram of the unedited render against the look's hues: same, complementary or triad), 30 % tonal key, 25 % content (skin the look would push, sky it suits). The score comes from the **before** render, so it does not move when you edit, and is the same on every run. `offer` is `natural` first, then every look scoring **60 or more**, at most **4**. Facet off is fine: the sky share is just 0.

## Snapshots

`draft_variant {id, name, score?, op}`: `save` (default) stores the photo's *live* recipe as a Develop snapshot named `AI: <name> · <score>` (through the page when the photo is open in Develop, so what you see is what is kept); `use` puts the newest snapshot called `AI: <name>` back as one history step. The user flips between looks by clicking the snapshots in Develop's sidebar. No new UI, no migration (`edit_history.snapshot_name`).

## Tools

Read: `list_photos`, `view` (`compare`, `region`, `overlay` = a mask id painted red), `analyze` (adds `regions` when Facet is on), `get_recipe`, `suggest_looks`. Write: `edit` (merge-patch), `mask` (linear, radial, ai; no brushes), `auto`, `undo`, `reset`, `apply_to`, `open_photo`, `draft_variant`.
