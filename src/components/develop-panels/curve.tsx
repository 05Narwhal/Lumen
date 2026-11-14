import { useState } from "react";
import { Segmented } from "@/components/segmented";
import { ToneCurve, type CurveChannel } from "@/components/tone-curve";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { CurvePoint } from "@/lib/tone-curve";
import type { Curve } from "@/types/generated/Curve";
import type { Develop } from "@/types/generated/Develop";
import { Panel } from "./panel";

const NAMES: Record<CurveChannel, string> = { master: "Curve", r: "Red curve", g: "Green curve", b: "Blue curve" };

const setChannel = (ch: CurveChannel, points: CurvePoint[]) => (d: Develop): Develop => ({ ...d, curve: { ...d.curve, [ch]: points } as Curve });

/** The tone curve (spec §1): a point curve on the master and on each of R, G, B. */
export function CurvePanel({ session }: { session: DevelopSession }) {
  const [channel, setChannelShown] = useState<CurveChannel>("master");
  const curve = session.develop.curve;
  const points = curve[channel] as CurvePoint[];
  return (
    <Panel
      id="curve"
      title="Tone curve"
      changed={JSON.stringify(curve) !== JSON.stringify(DEFAULT_DEVELOP.curve)}
      onReset={() => session.apply((d) => ({ ...d, curve: DEFAULT_DEVELOP.curve }), "Reset Tone curve")}
    >
      <div className="develop-panel__tabs develop-panel__tabs--row">
        <Segmented<CurveChannel>
          label="Channel"
          size="xs"
          value={channel}
          onChange={setChannelShown}
          data={[
            { value: "master", label: "Master" },
            { value: "r", label: "Red" },
            { value: "g", label: "Green" },
            { value: "b", label: "Blue" },
          ]}
        />
        <button
          type="button"
          className="develop-panel__link"
          disabled={points.length === 0}
          onClick={() => session.apply(setChannel(channel, []), `${NAMES[channel]} linear`)}
        >
          Linear
        </button>
      </div>
      <ToneCurve
        key={channel}
        channel={channel}
        label={NAMES[channel]}
        points={points}
        luma={session.histogram?.luma}
        onChange={(p) => session.update(setChannel(channel, p))}
        onCommit={(p, label) => {
          session.update(setChannel(channel, p));
          session.commit(label);
        }}
      />
    </Panel>
  );
}
