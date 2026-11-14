import { DevelopSlider } from "@/components/develop-slider";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { Basic } from "@/types/generated/Basic";
import { percent, stops } from "./format";
import { bind, Group, Panel, type PanelScope } from "./panel";

type Key = keyof Basic;

/** Changing Basic by hand means its values are no longer Auto's. */
const setBasic = (key: Key) => (d: import("@/types/generated/Develop").Develop, v: number) => ({ ...d, basic: { ...d.basic, [key]: v }, auto: false });

/** A ±1 control, shown ×100. */
function Unit({ session, k, label }: { session: DevelopSession; k: Key; label: string }) {
  return <DevelopSlider label={label} min={-100} max={100} format={percent} {...bind(session, (d) => d.basic[k], setBasic(k), 100)} />;
}

/** The Basic panel (spec §1): white balance, tone and presence. */
export function BasicPanel({ session, scope = "photo" }: { session: DevelopSession; scope?: PanelScope }) {
  const b = session.develop.basic;
  const wbChanged = b.temperature !== 0 || b.tint !== 0;
  return (
    <Panel
      id="basic"
      title="Basic"
      changed={JSON.stringify(b) !== JSON.stringify(DEFAULT_DEVELOP.basic)}
      onReset={() => session.apply((d) => ({ ...d, basic: DEFAULT_DEVELOP.basic, auto: false }), "Reset Basic")}
    >
      <Group
        title="White balance"
        action={
          // "As shot" is the photo's; a mask's white balance is only a shift, reset by its slider.
          scope === "mask" ? undefined : (
          <button
            type="button"
            className="develop-panel__link"
            disabled={!wbChanged}
            onClick={() => session.apply((d) => ({ ...d, basic: { ...d.basic, temperature: 0, tint: 0 }, auto: false }), "White balance as shot")}
          >
            As shot
          </button>
          )
        }
      >
        <DevelopSlider label="Temperature" min={-100} max={100} format={percent} track="var(--track-temperature)" {...bind(session, (d) => d.basic.temperature, setBasic("temperature"))} />
        <DevelopSlider label="Tint" min={-100} max={100} format={percent} track="var(--track-tint)" {...bind(session, (d) => d.basic.tint, setBasic("tint"))} />
      </Group>
      <Group title="Tone">
        <DevelopSlider label="Exposure" min={-5} max={5} step={0.05} format={stops} {...bind(session, (d) => d.basic.exposure, setBasic("exposure"))} />
        <Unit session={session} k="contrast" label="Contrast" />
        <Unit session={session} k="highlights" label="Highlights" />
        <Unit session={session} k="shadows" label="Shadows" />
        <Unit session={session} k="whites" label="Whites" />
        <Unit session={session} k="blacks" label="Blacks" />
      </Group>
      <Group title="Presence">
        <Unit session={session} k="texture" label="Texture" />
        <Unit session={session} k="clarity" label="Clarity" />
        <Unit session={session} k="dehaze" label="Dehaze" />
        <Unit session={session} k="vibrance" label="Vibrance" />
        <Unit session={session} k="saturation" label="Saturation" />
      </Group>
    </Panel>
  );
}
