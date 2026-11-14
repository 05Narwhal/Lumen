import { Slider } from "@mantine/core";
import { DevelopSlider } from "@/components/develop-slider";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { Develop } from "@/types/generated/Develop";
import type { Wheel } from "@/types/generated/Wheel";
import { ColorWheel } from "./color-wheel";
import { percent, whole } from "./format";
import { bind, Panel } from "./panel";

type Range = "shadows" | "midtones" | "highlights";
const RANGES: { key: Range; name: string }[] = [
  { key: "shadows", name: "Shadows" },
  { key: "midtones", name: "Midtones" },
  { key: "highlights", name: "Highlights" },
];

const setWheel = (key: Range, patch: Partial<Wheel>) => (d: Develop): Develop => ({ ...d, grading: { ...d.grading, [key]: { ...d.grading[key], ...patch } } });

/** Colour grading (spec §1): a tint for shadows, midtones and highlights, and how they blend. */
export function GradingPanel({ session }: { session: DevelopSession }) {
  const g = session.develop.grading;
  return (
    <Panel
      id="grading"
      title="Colour grading"
      changed={JSON.stringify(g) !== JSON.stringify(DEFAULT_DEVELOP.grading)}
      onReset={() => session.apply((d) => ({ ...d, grading: DEFAULT_DEVELOP.grading }), "Reset Colour grading")}
    >
      <div className="develop-panel__wheels">
        {RANGES.map(({ key, name }) => (
          <div key={key} className="develop-panel__wheel">
            <ColorWheel
              label={`${name} colour`}
              hue={g[key].hue}
              sat={g[key].sat}
              onChange={(hue, sat) => session.update(setWheel(key, { hue, sat }))}
              onCommit={(hue, sat, label) => {
                session.update(setWheel(key, { hue, sat }));
                session.commit(label);
              }}
            />
            <span className="develop-panel__wheel-name">{name}</span>
            <Slider
              className="develop-panel__wheel-lum"
              size="xs"
              min={-100}
              max={100}
              label={null}
              thumbLabel={`${name} luminance`}
              marks={[{ value: 0 }]}
              value={Math.round(g[key].lum * 100)}
              onChange={(v) => session.update(setWheel(key, { lum: v / 100 }))}
              onChangeEnd={(v) => {
                session.update(setWheel(key, { lum: v / 100 }));
                session.commit(`${name} luminance ${percent(v)}`);
              }}
            />
          </div>
        ))}
      </div>
      <DevelopSlider
        label="Blending"
        min={0}
        max={100}
        neutral={50}
        format={whole}
        {...bind(session, (d) => d.grading.blending, (d, v) => ({ ...d, grading: { ...d.grading, blending: v } }), 100)}
      />
      <DevelopSlider label="Balance" min={-100} max={100} format={percent} {...bind(session, (d) => d.grading.balance, (d, v) => ({ ...d, grading: { ...d.grading, balance: v } }), 100)} />
    </Panel>
  );
}
