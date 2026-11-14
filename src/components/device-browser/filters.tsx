import { Button, Chip, Switch, TextInput } from "@mantine/core";
import type { BrowserFilter } from "@/types/generated/BrowserFilter";
import type { BrowserSession } from "@/types/generated/BrowserSession";
import type { ItemKind } from "@/types/generated/ItemKind";
import { applyQuickPick, fromInputDate, QUICK_PICKS, toInputDate } from "./model";

const KINDS: { id: ItemKind; label: string }[] = [
  { id: "photo", label: "Photos" },
  { id: "video", label: "Videos" },
  { id: "raw", label: "RAW" },
  { id: "live", label: "Live" },
];

/**
 * The browser's top bar: a date range from–to with quick picks, kinds, *New only* (when the
 * source knows what it gave the library before) and whether undated items stay in a range.
 */
export function Filters({
  session,
  filter,
  onChange,
  now = () => new Date(),
}: {
  session: BrowserSession | null;
  filter: BrowserFilter;
  onChange: (f: BrowserFilter) => void;
  now?: () => Date;
}) {
  const ranged = filter.from !== null || filter.to !== null;
  return (
    <div className="device-browser__filters" role="group" aria-label="Filters">
      <div className="device-browser__range">
        <TextInput
          type="date"
          size="xs"
          label="From"
          value={toInputDate(filter.from)}
          onChange={(e) => onChange({ ...filter, from: fromInputDate(e.currentTarget.value) })}
        />
        <TextInput
          type="date"
          size="xs"
          label="To"
          value={toInputDate(filter.to)}
          onChange={(e) => onChange({ ...filter, to: fromInputDate(e.currentTarget.value, true) })}
        />
      </div>
      <div className="device-browser__picks">
        {QUICK_PICKS.filter((p) => p.id !== "new" || session?.knowsImported !== false).map((p) => (
          <Button key={p.id} size="compact-xs" variant="subtle" onClick={() => onChange(applyQuickPick(filter, p.id, now()))}>
            {p.label}
          </Button>
        ))}
      </div>
      <Chip.Group multiple value={filter.kinds} onChange={(v) => onChange({ ...filter, kinds: v as ItemKind[] })}>
        <div className="device-browser__kinds" aria-label="Kinds">
          {KINDS.map((k) => (
            <Chip key={k.id} value={k.id} size="xs">
              {k.label}
            </Chip>
          ))}
        </div>
      </Chip.Group>
      {session?.knowsImported !== false && (
        <Switch size="xs" label="New only" checked={filter.newOnly} onChange={(e) => onChange({ ...filter, newOnly: e.currentTarget.checked })} />
      )}
      <Switch
        size="xs"
        label="Include undated"
        disabled={!ranged}
        checked={filter.includeUndated}
        onChange={(e) => onChange({ ...filter, includeUndated: e.currentTarget.checked })}
      />
    </div>
  );
}
