import { Button } from "@mantine/core";
import { IconSparkles } from "@tabler/icons-react";
import { AiButton } from "@/components/ai-button";
import { DevelopSlider } from "@/components/develop-slider";
import { useDenoise } from "@/hooks/use-denoise";
import type { DevelopSession } from "@/hooks/use-develop-session";
import type { DenoiseFinished } from "@/types/generated/DenoiseFinished";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { Develop } from "@/types/generated/Develop";
import type { Grain } from "@/types/generated/Grain";
import type { Sharpen } from "@/types/generated/Sharpen";
import type { Vignette } from "@/types/generated/Vignette";
import { degrees, percent, pixels, whole } from "./format";
import { bind, Group, Panel, type PanelScope } from "./panel";

const sharpen = (k: keyof Sharpen) => (d: Develop, v: number): Develop => ({ ...d, detail: { ...d.detail, sharpen: { ...d.detail.sharpen, [k]: v } } });
const vignette = (k: keyof Vignette) => (d: Develop, v: number): Develop => ({ ...d, effects: { ...d.effects, vignette: { ...d.effects.vignette, [k]: v } } });
const grain = (k: keyof Grain) => (d: Develop, v: number): Develop => ({ ...d, effects: { ...d.effects, grain: { ...d.effects.grain, [k]: v } } });
// Off, these three are left out of the recipe (as Rust writes it), so an old recipe stays byte for byte.
const OFF = { motion_blur: { amount: 0, angle: 0 }, glow: { amount: 0, size: 0.4 }, halation: { amount: 0, size: 0.3 } };
type Optional = keyof typeof OFF;
const optional =
  <K extends Optional>(key: K, k: keyof (typeof OFF)[K]) =>
  (d: Develop, v: number): Develop => {
    const next = { ...OFF[key], ...d.effects[key], [k]: v };
    const effects = { ...d.effects, [key]: next };
    if (JSON.stringify(next) === JSON.stringify(OFF[key])) delete effects[key];
    return { ...d, effects };
  };
const read = <K extends Optional>(key: K, k: keyof (typeof OFF)[K]) => (d: Develop) => ((d.effects[key] ?? OFF[key]) as Record<string, number>)[k as string];

/**
 * Denoise (Phase 8d, spec §4): run on demand, with progress and Cancel; once built, a strength
 * slider and Remove. A recipe that asks for denoise whose base is missing (pasted, cache
 * cleared) offers Rebuild. A failure is said here, not in a dialog.
 */
function DenoiseGroup({ session }: { session: DevelopSession }) {
  const current = session.develop.detail.denoise;
  const onFinished = (f: DenoiseFinished) => {
    if (!f.ok) return;
    if (session.develop.detail.denoise) session.refresh();
    else session.apply((d) => ({ ...d, detail: { ...d.detail, denoise: { strength: 0.5, model: f.model } } }), "Denoise");
  };
  const { status, progress, error, slow, start, cancel } = useDenoise(session.assetId, onFinished);
  const running = progress !== null;
  return (
    <Group title="Denoise">
      {running ? (
        <div className="develop-panel__denoise">
          {/* The button is the progress: a ring on its border, Cancel on hover or a click. */}
          <AiButton
            label="Denoise"
            icon={<IconSparkles size={14} />}
            state={{ visible: true, stage: "Denoising", done: Math.round(progress * 100), total: 100 }}
            onRun={() => {}}
            onCancel={() => void cancel()}
          />
          {slow && (
            <>
              <p className="develop-panel__hint">Denoise is taking longer than usual.</p>
              <Button size="compact-sm" variant="subtle" onClick={() => void cancel()}>
                Cancel
              </Button>
            </>
          )}
        </div>
      ) : current ? (
        <>
          <DevelopSlider
            label="Strength"
            historyLabel="Denoise strength"
            min={0}
            max={100}
            neutral={50}
            format={whole}
            {...bind(session, (d) => d.detail.denoise?.strength ?? 0, (d, v) => ({ ...d, detail: { ...d.detail, denoise: d.detail.denoise ? { ...d.detail.denoise, strength: v } : null } }), 100)}
          />
          {status && !status.ready && (
            <div className="develop-panel__denoise">
              <p className="develop-panel__hint">Denoise needs to be rebuilt for this photo.</p>
              <AiButton label="Rebuild" state={null} disabled={!status.available} onRun={() => void start()} onCancel={() => {}} />
            </div>
          )}
          <div className="develop-panel__denoise">
            <Button size="compact-sm" variant="subtle" onClick={() => session.apply((d) => ({ ...d, detail: { ...d.detail, denoise: null } }), "Remove denoise")}>
              Remove
            </Button>
          </div>
        </>
      ) : (
        <div className="develop-panel__denoise">
          <AiButton
            label="Denoise"
            icon={<IconSparkles size={14} />}
            state={null}
            disabled={!status?.available}
            onRun={() => void start()}
            onCancel={() => {}}
          />
          {status && !status.available && status.reason && <p className="develop-panel__hint">{status.reason}</p>}
        </div>
      )}
      {error && (
        <div className="develop-panel__denoise">
          <p className="develop-panel__hint develop-panel__hint--error">{error}</p>
        </div>
      )}
    </Group>
  );
}

/** Detail (spec §1): sharpening, and Phase 8d's ML denoise. */
export function DetailPanel({ session, scope = "photo" }: { session: DevelopSession; scope?: PanelScope }) {
  const s = DEFAULT_DEVELOP.detail.sharpen;
  const mask = scope === "mask";
  return (
    <Panel
      id="detail"
      title="Detail"
      changed={JSON.stringify(session.develop.detail.sharpen) !== JSON.stringify(s)}
      onReset={() => session.apply((d) => ({ ...d, detail: { ...d.detail, sharpen: s } }), "Reset Detail")}
    >
      <Group title="Sharpening">
        {/* A mask's amount is added to the photo's, so it can also soften. */}
        <DevelopSlider label="Amount" historyLabel="Sharpening" min={mask ? -100 : 0} max={150} neutral={0} format={mask ? percent : whole} {...bind(session, (d) => d.detail.sharpen.amount, sharpen("amount"), 100)} />
        <DevelopSlider label="Radius" historyLabel="Sharpening radius" min={0.5} max={3} step={0.1} neutral={s.radius} format={pixels} {...bind(session, (d) => d.detail.sharpen.radius, sharpen("radius"))} />
        <DevelopSlider label="Detail" historyLabel="Sharpening detail" min={0} max={100} neutral={s.detail * 100} format={whole} {...bind(session, (d) => d.detail.sharpen.detail, sharpen("detail"), 100)} />
        <DevelopSlider label="Masking" historyLabel="Sharpening masking" min={0} max={100} neutral={0} format={whole} {...bind(session, (d) => d.detail.sharpen.masking, sharpen("masking"), 100)} />
      </Group>
      {!mask && <DenoiseGroup session={session} />}
    </Panel>
  );
}

/** Effects (spec §1): motion blur, glow, halation, a vignette on the cropped frame, and film grain. */
export function EffectsPanel({ session, scope = "photo" }: { session: DevelopSession; scope?: PanelScope }) {
  const e = DEFAULT_DEVELOP.effects;
  return (
    <Panel
      id="effects"
      title="Effects"
      changed={JSON.stringify(session.develop.effects) !== JSON.stringify(e)}
      onReset={() => session.apply((d) => ({ ...d, effects: e }), "Reset Effects")}
    >
      <Group title="Motion blur">
        <DevelopSlider label="Amount" historyLabel="Motion blur" min={0} max={100} neutral={0} format={whole} {...bind(session, read("motion_blur", "amount"), optional("motion_blur", "amount"), 100)} />
        <DevelopSlider label="Direction" historyLabel="Motion blur direction" min={-90} max={90} neutral={0} format={degrees} {...bind(session, read("motion_blur", "angle"), optional("motion_blur", "angle"), 1)} />
      </Group>
      <Group title="Glow">
        <DevelopSlider label="Amount" historyLabel="Glow" min={0} max={100} neutral={0} format={whole} {...bind(session, read("glow", "amount"), optional("glow", "amount"), 100)} />
        <DevelopSlider label="Size" historyLabel="Glow size" min={0} max={100} neutral={OFF.glow.size * 100} format={whole} {...bind(session, read("glow", "size"), optional("glow", "size"), 100)} />
      </Group>
      <Group title="Halation">
        <DevelopSlider label="Amount" historyLabel="Halation" min={0} max={100} neutral={0} format={whole} {...bind(session, read("halation", "amount"), optional("halation", "amount"), 100)} />
        <DevelopSlider label="Size" historyLabel="Halation size" min={0} max={100} neutral={OFF.halation.size * 100} format={whole} {...bind(session, read("halation", "size"), optional("halation", "size"), 100)} />
      </Group>
      {scope === "photo" && (
        <Group title="Vignette">
          <DevelopSlider label="Amount" historyLabel="Vignette" min={-100} max={100} format={percent} {...bind(session, (d) => d.effects.vignette.amount, vignette("amount"), 100)} />
          <DevelopSlider label="Midpoint" historyLabel="Vignette midpoint" min={0} max={100} neutral={e.vignette.midpoint * 100} format={whole} {...bind(session, (d) => d.effects.vignette.midpoint, vignette("midpoint"), 100)} />
          <DevelopSlider label="Feather" historyLabel="Vignette feather" min={0} max={100} neutral={e.vignette.feather * 100} format={whole} {...bind(session, (d) => d.effects.vignette.feather, vignette("feather"), 100)} />
        </Group>
      )}
      <Group title="Grain">
        <DevelopSlider label="Amount" historyLabel="Grain" min={0} max={100} neutral={0} format={whole} {...bind(session, (d) => d.effects.grain.amount, grain("amount"), 100)} />
        <DevelopSlider label="Size" historyLabel="Grain size" min={0} max={100} neutral={e.grain.size * 100} format={whole} {...bind(session, (d) => d.effects.grain.size, grain("size"), 100)} />
      </Group>
    </Panel>
  );
}
