import { useState } from "react";
import { DevelopSlider } from "@/components/develop-slider";
import { Segmented } from "@/components/segmented";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { Develop } from "@/types/generated/Develop";
import { percent } from "./format";
import { bind, Panel } from "./panel";

/** In `HSL_BANDS` order (photos-core::edits). */
export const BANDS = [
  { name: "Red", token: "var(--band-red)" },
  { name: "Orange", token: "var(--band-orange)" },
  { name: "Yellow", token: "var(--band-yellow)" },
  { name: "Green", token: "var(--band-green)" },
  { name: "Aqua", token: "var(--band-aqua)" },
  { name: "Blue", token: "var(--band-blue)" },
  { name: "Purple", token: "var(--band-purple)" },
  { name: "Magenta", token: "var(--band-magenta)" },
] as const;

type Part = "h" | "s" | "l";
const PART_NAME: Record<Part, string> = { h: "hue", s: "saturation", l: "luminance" };

const setBand = (i: number, part: Part) => (d: Develop, v: number): Develop => {
  const hsl = [...d.hsl] as Develop["hsl"];
  hsl[i] = { ...hsl[i], [part]: v };
  return { ...d, hsl };
};

/** HSL (spec §1): eight overlapping hue bands, one of hue, saturation or luminance at a time. */
export function HslPanel({ session }: { session: DevelopSession }) {
  const [part, setPart] = useState<Part>("h");
  return (
    <Panel
      id="hsl"
      title="Colour"
      changed={JSON.stringify(session.develop.hsl) !== JSON.stringify(DEFAULT_DEVELOP.hsl)}
      onReset={() => session.apply((d) => ({ ...d, hsl: DEFAULT_DEVELOP.hsl }), "Reset Colour")}
    >
      <div className="develop-panel__tabs">
        <Segmented<Part>
          label="Adjust"
          size="xs"
          value={part}
          onChange={setPart}
          data={[
            { value: "h", label: "Hue" },
            { value: "s", label: "Saturation" },
            { value: "l", label: "Luminance" },
          ]}
        />
      </div>
      {BANDS.map((band, i) => (
        <DevelopSlider
          key={`${part}-${band.name}`}
          label={band.name}
          historyLabel={`${band.name} ${PART_NAME[part]}`}
          adornment={<span className="develop-panel__swatch" style={{ background: band.token }} aria-hidden />}
          min={-100}
          max={100}
          format={percent}
          {...bind(session, (d) => d.hsl[i][part], setBand(i, part), 100)}
        />
      ))}
    </Panel>
  );
}
