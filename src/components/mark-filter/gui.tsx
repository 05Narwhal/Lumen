import { IconFlag, IconFlagFilled, IconFlagOff } from "@tabler/icons-react";
import { FLAG_NONE, FLAG_PICK, FLAG_REJECT, LABELS, flagName } from "@/lib/marks";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import "./styles.scss";

const FLAGS = [
  { value: FLAG_PICK, Icon: IconFlagFilled },
  { value: FLAG_REJECT, Icon: IconFlagOff },
  { value: FLAG_NONE, Icon: IconFlag },
] as const;

/**
 * The culling toolbar: one click shows only the picks, the rejects or the unflagged, and
 * the colour dots show any of the labels chosen. Marks belong to Develop (8c), whose top
 * bar carries this; the library does not show them.
 */
export function MarkFilter({ query, onChange }: { query: AssetQuery; onChange: (patch: Partial<AssetQuery>) => void }) {
  const labels = query.colorLabels;
  return (
    <div className="mark-filter" role="toolbar" aria-label="Flags and labels">
      {FLAGS.map(({ value, Icon }) => {
        const on = query.flag === value;
        return (
          <button
            key={value}
            type="button"
            className="mark-filter__flag"
            data-flag={value}
            aria-pressed={on}
            aria-label={`Show ${flagName(value).toLowerCase()}`}
            title={flagName(value)}
            onClick={() => onChange({ flag: on ? null : value })}
          >
            <Icon size={14} aria-hidden />
          </button>
        );
      })}
      <span className="mark-filter__rule" aria-hidden />
      {LABELS.map((l) => {
        const on = labels.includes(l.value);
        return (
          <button
            key={l.value}
            type="button"
            className="mark-filter__dot"
            style={{ ["--dot" as string]: l.color }}
            aria-pressed={on}
            aria-label={`Show ${l.name.toLowerCase()} label`}
            title={l.name}
            onClick={() => onChange({ colorLabels: on ? labels.filter((v) => v !== l.value) : [...labels, l.value].sort() })}
          />
        );
      })}
    </div>
  );
}
