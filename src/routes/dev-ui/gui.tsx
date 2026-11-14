import { Button, SegmentedControl, Switch } from "@mantine/core";
import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { AiButton } from "@/components/ai-button";
import { ConfirmDialogHost, confirm } from "@/components/confirm-dialog";
import { FilterChips } from "@/components/filter-chips";
import { MiniMap } from "@/components/mini-map";
import { MusicPicker } from "@/components/music-picker";
import { PageHeader } from "@/components/page-header";
import { SmartAlbumEditor } from "@/components/smart-album-editor";
import { Spinner } from "@/components/spinner";
import { StarRating } from "@/components/star-rating";
import { clearSelection, selectAll } from "@/hooks/use-selection";
import { DeviceBrowser } from "@/components/device-browser";
import { PhotoPicker } from "@/components/photo-picker";
import { EMPTY_QUERY } from "@/hooks/use-search";
import "./styles.scss";
// Loads the Settings stylesheet, which the Discord card below borrows (dev only).
import "@/routes/settings";
import { DiscordSection } from "@/routes/settings/discord";
import { openExportLosses, openExportSheet } from "@/components/export-sheet";
import { EditToolbar, TextOverlay, TextSection } from "@/components/viewer";
import { TextEmptyState } from "@/components/text-empty";
import { SplitButton } from "@/components/split-button";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";
import { IDENTITY, type EditSession } from "@/hooks/use-edit-session";
import type { Recipe } from "@/types/generated/Recipe";
import type { UpdateState } from "@/types/generated/UpdateState";
import { updateLine } from "@/routes/settings/updates";
import { PluginInstallSheet } from "@/components/plugin-install";
import { PluginsView } from "@/routes/settings/plugins";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { PluginInspect } from "@/types/generated/PluginInspect";

const PLUGIN_INFO: PluginInfo = {
  id: "studio.jl.takeout",
  name: "Google Takeout",
  version: "1.2.0",
  author: "JL Studios",
  description: "Imports a Google Takeout export.",
  license: "MIT",
  trust: { type: "jlStudios" },
  kinds: ["importer"],
  permissions: ["importReadChosen", "storage"],
  enabled: true,
  dev: false,
  state: { type: "ready" },
  iconUrl: null,
  settings: [],
  assets: [],
};
const DATA_ONLY: PluginInfo = { ...PLUGIN_INFO, id: "studio.jl.ember", name: "Ember", kinds: ["theme"], permissions: [], license: "CC-BY-4.0" };
const inspect = (over: Partial<PluginInspect> = {}, info: Partial<PluginInfo> = {}): PluginInspect => ({
  info: { ...PLUGIN_INFO, ...info },
  adds: ["An importer: Google Takeout"],
  problems: [],
  notes: [],
  update: null,
  older: null,
  ...over,
});
const SHEET_STATES: { name: string; data: PluginInspect }[] = [
  { name: "New", data: inspect() },
  { name: "Data only", data: inspect({ adds: ["A theme: Ember"] }, DATA_ONLY) },
  { name: "Update", data: inspect({ update: { from: "1.1.0", newPermissions: ["storage"] } }) },
  { name: "Older", data: inspect({ older: "1.3.0" }) },
  { name: "Problem", data: inspect({ problems: ["manifest.toml: unknown permission \"network\""] }) },
  { name: "Unsigned", data: inspect({ notes: ["Anyone can sign a plugin; this one is not signed."] }, { trust: { type: "unsigned" } }) },
];
const PLUGIN_ROWS: PluginInfo[] = [PLUGIN_INFO, DATA_ONLY, { ...PLUGIN_INFO, id: "dev.local", name: "Local test", dev: true, trust: { type: "unsigned" }, state: { type: "problem", reason: "Bad manifest" } }];

const UPDATE_STATES: UpdateState[] = [
  { state: "off" },
  { state: "notSetUp" },
  { state: "idle" },
  { state: "checking" },
  { state: "upToDate", checkedAt: 1_790_000_000 },
  { state: "available", version: "26.1.1-alpha", notes: "" },
  { state: "downloading", version: "26.1.1-alpha", done: 42, total: 100 },
  { state: "ready", version: "26.1.1-alpha", notes: "", attempts: 0 },
  { state: "ready", version: "26.1.1-alpha", notes: "", attempts: 1 },
  { state: "failed", why: "offline", at: 1 },
  { state: "failed", why: "noServer", at: 1 },
  { state: "failed", why: "couldNotCheck", at: 1 },
  { state: "failed", why: "badSignature", at: 1 },
  { state: "failed", why: "noSpace", at: 1 },
];
import type { Settings } from "@/types/generated/Settings";

const DISCORD_OFF = { discord: { enabled: false, showActivity: true, showElapsed: true } } as Settings;
const DISCORD_ON = { discord: { enabled: true, showActivity: true, showElapsed: true } } as Settings;

/**
 * Dev-only surface (never routed in a release build): every small control and every modal
 * on one page, so a screenshot answers "does this animate, is that opaque" without
 * clicking through the app.
 */
/**
 * Reports what the browser actually computed, and whether a transition really ran.
 *
 * Computed style only proves a transition is *declared*. This also starts a real one on a
 * throwaway element and waits for `transitionend`, whose `elapsedTime` is the browser
 * telling us how long it actually animated for. That is the one check that survives every
 * environment — including a headless browser, where sampling positions across frames
 * reports no movement even when the animation is running perfectly.
 */
function ComputedProbe() {
  const [rows, setRows] = useState<string[]>([]);
  const canary = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const read = (sel: string, props: string[]) => {
      const el = document.querySelector(sel);
      if (!el) return `${sel}: MISSING`;
      const cs = getComputedStyle(el);
      return `${sel}: ${props.map((p) => `${p}=${cs.getPropertyValue(p)}`).join(" ")}`;
    };

    const verdict = new Promise<string>((resolve) => {
      const el = canary.current;
      if (!el) return resolve("motion-check: NO ELEMENT");
      const done = (e: TransitionEvent) => {
        el.removeEventListener("transitionend", done);
        resolve(`motion-check: transitionend after ${Math.round(e.elapsedTime * 1000)}ms — ANIMATION RAN`);
      };
      el.addEventListener("transitionend", done);
      requestAnimationFrame(() => {
        el.style.transform = "translateX(120px)";
      });
      setTimeout(
        () =>
          resolve(
            "motion-check: NO transitionend in 1.5s = ANIMATION DID NOT RUN." +
              " In a headless browser this is expected (it does not advance the compositor" +
              " clock). In the real app window it means motion is genuinely broken.",
          ),
        1500,
      );
    });

    void verdict.then((v) =>
      setRows([
        `build=${typeof __BUILD_COMMIT__ === "string" ? __BUILD_COMMIT__ : "unknown"}`,
        v,
        read(".mantine-Switch-thumb", ["transition-property", "transition-duration", "transform"]),
        read(".mantine-Switch-track", ["transition-property", "transition-duration"]),
        read(".mantine-SegmentedControl-indicator", ["transition-property", "transition-duration"]),
        read(":root", ["--motion-base", "--motion-fast", "--motion-slow", "--ease"]),
        `reduced-motion-attr=${document.documentElement.dataset.reducedMotion} prefers-reduced-motion=${window.matchMedia("(prefers-reduced-motion: reduce)").matches}`,
      ]),
    );
  }, []);

  return (
    <>
      {/* The canary: a real element running a real transition. */}
      <span
        ref={canary}
        className="dev-ui__canary"
        aria-hidden
        style={{ transition: "transform var(--motion-slow) var(--ease)" }}
      />
      <pre data-testid="computed" className="dev-ui__probe">
        {rows.join(String.fromCharCode(10))}
      </pre>
    </>
  );
}

/** The quick editor's toolbar on a local draft: every control, no library needed. */
function EditToolbarProbe() {
  const [draft, setDraft] = useState<Recipe>({ ...IDENTITY, straighten: 2.5, flip_v: true });
  const [cropping, setCropping] = useState(true);
  const [aspect, setAspect] = useState<number | null | "original">(1);
  const session: EditSession = {
    draft,
    saved: null,
    loaded: true,
    previewUrl: null,
    rendering: false,
    dirty: true,
    set: (patch) => setDraft((d) => (typeof patch === "function" ? patch(d) : { ...d, ...patch })),
    reset: (key) => setDraft((d) => ({ ...d, [key]: IDENTITY[key] })),
    undo: () => {},
    redo: () => {},
    canUndo: true,
    canRedo: false,
    done: async () => {},
    cancel: () => setDraft(IDENTITY),
    revert: async () => {},
  };
  const [grid, setGrid] = useState<"auto" | "thirds" | "fine" | "off">("thirds");
  return (
    <EditToolbar
      session={session}
      cropping={cropping}
      onCropping={setCropping}
      aspect={aspect}
      onAspect={setAspect}
      grid={grid}
      onGrid={setGrid}
      onStraightening={() => {}}
      cropSize={cropping ? { width: 3024, height: 3024 } : null}
    />
  );
}

/** Three lines as `asset_text` would return them: level, turned a little, and upright. */
const TEXT_LINES: ShownTextLine[] = [
  { ord: 0, text: "Gostilna pri Mostu", quad: [0.08, 0.12, 0.62, 0.12, 0.62, 0.24, 0.08, 0.24] },
  { ord: 1, text: "Kosilo 12 €", quad: [0.12, 0.42, 0.5, 0.36, 0.515, 0.46, 0.135, 0.52] },
  { ord: 2, text: "ODPRTO", quad: [0.86, 0.15, 0.86, 0.75, 0.78, 0.75, 0.78, 0.15] },
];

/** A sign and a menu, twelve lines: enough for the inset's two sentences and a scrolling *Show all*. */
const DEV_PARAGRAPH: ShownTextLine[] = [
  "Gostilna pri Mostu",
  "Odprto vsak dan od 11. do 22. ure.",
  "Dnevno kosilo 12 €",
  "Juha, glavna jed",
  "in sladica.",
  "Rezervacije",
  "po telefonu ali",
  "na naslovu info@example.com",
  "Plačilo samo z gotovino.",
  "Hvala za obisk!",
  "Vrata so na dvorišču.",
  "Pes je dobrodošel.",
].map((text, ord) => {
  const y = 0.05 + ord * 0.06 + (ord > 4 ? 0.05 : 0);
  return { ord, text, quad: [0.1, y, 0.9, y, 0.9, y + 0.05, 0.1, y + 0.05] as ShownTextLine["quad"] };
});

export function DevUiRoute() {
  // The selection bar only exists while something is selected, so seed a selection here:
  // it is the one control whose narrow-window behaviour cannot be seen otherwise. The
  // shell clears the selection on every navigation, and that effect runs after this one,
  // so the seed waits a tick for it.
  useEffect(() => {
    const t = setTimeout(() => selectAll([1, 2, 3]), 0);
    return () => {
      clearTimeout(t);
      clearSelection();
    };
  }, []);
  const [checked, setChecked] = useState(true);
  const [size, setSize] = useState("m");
  const [rating, setRating] = useState(3);
  const [smart, setSmart] = useState(false);
  const [music, setMusic] = useState(false);
  const [picker, setPicker] = useState(false);
  const [device, setDevice] = useState(false);
  // `?modal=smart|confirm|picker|device` opens one on load, so a headless screenshot can see it.
  const [params] = useSearchParams();
  const which = params.get("modal");
  useEffect(() => {
    if (which === "smart") setSmart(true);
    // The device browser over the shim's 6,000-item card (Round 4E).
    if (which === "device") setDevice(true);
    if (which === "music") setMusic(true);
    // The photo picker with three photos chosen, so its confirm button is enabled
    // (`&none=1` leaves it empty to show the disabled state).
    if (which === "picker") {
      setPicker(true);
      if (params.get("none") !== "1") setTimeout(() => selectAll([1, 2, 3]), 300);
    }
    if (which === "export") openExportSheet([1, 2, 3]);
    if (which === "losses")
      openExportLosses([
        { assetId: 1, file: "DSC_0676-edited.avif", lost: [{ tag: "LensModel", before: "NIKKOR Z 24-70mm f/4 S", after: "" }, { tag: "SubSecTimeOriginal", before: "42", after: "" }] },
        { assetId: 2, file: "IMG_0412.webp", lost: [{ tag: "GPSAltitude", before: "295", after: "294.9" }] },
      ]);
    if (which === "confirm") void confirm({ title: "Delete 12 items permanently?", body: "This cannot be undone.", confirmLabel: "Delete now", danger: true });
  }, [which, params]);

  return (
    <div className="dev-ui">
      {device && <DeviceBrowser target={{ kind: "volume", id: "E:\\" }} title="EOS_DIGITAL" onClose={() => setDevice(false)} />}
      {picker && <PhotoPicker title="Choose photos to develop" confirmLabel="Develop" onClose={() => setPicker(false)} onPick={() => setPicker(false)} />}
      <PageHeader title="UI parts" search={false} />
      <div className="dev-ui__grid">
        <section className="dev-ui__card">
          <h2>Controls</h2>
          <Switch checked={checked} onChange={(e) => setChecked(e.currentTarget.checked)} label="Switch on" />
          <Switch checked={false} onChange={() => {}} label="Switch off" />
          <SegmentedControl
            value={size}
            onChange={setSize}
            data={[
              { label: "S", value: "s" },
              { label: "M", value: "m" },
              { label: "L", value: "l" },
            ]}
          />
          <StarRating value={rating} onChange={setRating} />
          <Spinner size="md" label="Working" />
          <SplitButton label="New album" onClick={() => {}} menu={[{ label: "New smart album", onClick: () => {} }]} />
        </section>

        <section className="dev-ui__card">
          <h2>AI button</h2>
          <AiButton label="Auto" state={null} onRun={() => {}} onCancel={() => {}} />
          <AiButton label="Auto" state={{ visible: true, stage: "Reading photo…", done: 0, total: 0 }} onRun={() => {}} onCancel={() => {}} />
          <AiButton label="Auto" state={{ visible: true, stage: "Trying looks 2/3", done: 2, total: 3 }} onRun={() => {}} onCancel={() => {}} />
          <AiButton label="Auto" state={{ visible: true, stage: "Applying…", done: 9, total: 10 }} onRun={() => {}} onCancel={() => {}} />
          <AiButton label="Auto" state={null} disabled onRun={() => {}} onCancel={() => {}} />
          <AiButton label="Finding masks" state={{ visible: true, stage: "Finding masks…", done: 0, total: 0 }} onRun={() => {}} onCancel={() => {}} />
        </section>

        <section className="dev-ui__card">
          <h2>Filters</h2>
          <FilterChips query={{ ...EMPTY_QUERY, minRating: 4, favorites: true }} onChange={() => {}} />
        </section>

        {/* Round 13-3: the text overlay on a stand-in photo whose words are drawn with the
            same geometry (so the outlines must sit on them), the info's Text section while
            reading and with lines, the Has text empty state and chip. */}
        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Text in photos</h2>
          <div className="dev-ui__text">
            <div className="dev-ui__text-photo">
              <div className="dev-ui__text-words" aria-hidden>
                <TextOverlay lines={TEXT_LINES} aspect={1.5} onMenu={() => {}} />
              </div>
              <TextOverlay lines={TEXT_LINES} aspect={1.5} onMenu={() => {}} />
            </div>
            <div className="dev-ui__text-side">
              <TextSection lines={DEV_PARAGRAPH} reading={false} shown={false} onShowOnPhoto={() => {}} onMenu={() => {}} />
              <TextSection lines={TEXT_LINES} reading={false} shown onShowOnPhoto={() => {}} onMenu={() => {}} />
              <TextSection lines={[]} reading shown={false} onShowOnPhoto={() => {}} onMenu={() => {}} />
              <FilterChips query={{ ...EMPTY_QUERY, hasText: true }} onChange={() => {}} />
            </div>
          </div>
          <div className="dev-ui__text-empty">
            <TextEmptyState />
          </div>
        </section>

        {/* Round 17: every line Settings → Updates can show (the section itself: `?update=<state>`). */}
        <section className="dev-ui__card">
          <h2>Updates</h2>
          <ul className="dev-ui__list">
            {UPDATE_STATES.map((s, i) => (
              <li key={i}>
                <code>{s.state}</code> {updateLine(s)}
              </li>
            ))}
          </ul>
        </section>

        {/* Round 17: Settings → Discord status, off and on. */}
        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Discord status</h2>
          <div className="settings">
            <DiscordSection settings={DISCORD_OFF} update={() => {}} />
            <DiscordSection settings={DISCORD_ON} update={() => {}} />
          </div>
        </section>

        <section className="dev-ui__card">
          <h2>Computed</h2>
          <ComputedProbe />
        </section>

        {/* The base map at the three detail levels it switches between: the 110 m world,
            the 50 m set with relief, and a close view where only the coastline is left. */}
        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Base map</h2>
          <div className="dev-ui__maps">
            <MiniMap lat={25} lon={10} zoom={1.2} width={420} height={260} />
            <MiniMap lat={46.3} lon={10.5} zoom={5} width={420} height={260} />
            <MiniMap lat={46.36} lon={14.1} zoom={9} width={420} height={260} />
          </div>
        </section>

        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Edit toolbar</h2>
          <EditToolbarProbe />
        </section>

        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Plugin install sheet</h2>
          {SHEET_STATES.map((s) => (
            <div key={s.name}>
              <h3>{s.name}</h3>
              <div className="plugin-install" style={{ padding: "var(--space-4)", maxWidth: 480 }}>
                <div className="plugin-install__body">
                  <PluginInstallSheet p={s.data} busy={false} error={null} onInstall={() => {}} />
                </div>
              </div>
            </div>
          ))}
        </section>

        <section className="dev-ui__card dev-ui__card--wide">
          <h2>Plugins page, empty</h2>
          <PluginsView plugins={[]} developerMode={false} reload={() => {}} />
          <h2>Plugins page, filled</h2>
          <PluginsView plugins={PLUGIN_ROWS} developerMode reload={() => {}} />
        </section>

        <section className="dev-ui__card">
          <h2>Modals</h2>
          <Button onClick={() => setSmart(true)}>Smart album editor</Button>
          <Button
            variant="default"
            onClick={() => void confirm({ title: "Delete 12 items permanently?", body: "This cannot be undone.", confirmLabel: "Delete now", danger: true })}
          >
            Confirm dialog
          </Button>
          <Button variant="default" onClick={() => openExportSheet([1, 2, 3])}>
            Export sheet
          </Button>
        </section>
      </div>
      {smart && <SmartAlbumEditor draft={{ name: "Best of 2026", query: EMPTY_QUERY }} onClose={() => setSmart(false)} onSave={() => setSmart(false)} />}
      {music && <MusicPicker initialTab={params.get("tab") ?? undefined} onClose={() => setMusic(false)} onUse={() => setMusic(false)} onChooseFile={() => {}} />}
      <ConfirmDialogHost />
    </div>
  );
}
