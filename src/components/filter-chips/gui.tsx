import { Button, Checkbox, Popover, Select, TextInput } from "@mantine/core";
import { IconAdjustments, IconX } from "@tabler/icons-react";
import { useState } from "react";
import { StarRating } from "@/components/star-rating";
import { useDevices } from "@/hooks/use-devices";
import { useSettings } from "@/hooks/use-settings";
import { readTextOn } from "@/lib/photo-text";
import { KIND_LABELS } from "@/lib/collections";
import type { AssetKind } from "@/types/generated/AssetKind";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import { formatCapturedDate } from "@/utils/date";
import "./styles.scss";

export interface FilterChip {
  key: string;
  label: string;
  clear: Partial<AssetQuery>;
}

/** One readable chip per active filter, in the order the user is likely to scan. */
export function chipsFor(q: AssetQuery, deviceName: (id: number) => string): FilterChip[] {
  const chips: FilterChip[] = [];
  if (q.dateFrom != null || q.dateTo != null) {
    const from = q.dateFrom != null ? formatCapturedDate(q.dateFrom) : "any";
    const to = q.dateTo != null ? formatCapturedDate(q.dateTo) : "now";
    chips.push({ key: "dateFrom", label: `${from} – ${to}`, clear: { dateFrom: null, dateTo: null } });
  }
  for (const id of q.deviceIds) {
    chips.push({ key: "deviceIds", label: deviceName(id), clear: { deviceIds: q.deviceIds.filter((d) => d !== id) } });
  }
  if (q.mediaType != null) {
    chips.push({ key: "mediaType", label: q.mediaType === 1 ? "Videos" : "Photos", clear: { mediaType: null } });
  }
  if (q.kind) chips.push({ key: "kind", label: KIND_LABELS[q.kind], clear: { kind: null } });
  if (q.favorites) chips.push({ key: "favorites", label: "Favorites", clear: { favorites: false } });
  if (q.minRating) chips.push({ key: "minRating", label: `${q.minRating}+ stars`, clear: { minRating: null } });
  if (q.hasGps != null) {
    chips.push({ key: "hasGps", label: q.hasGps ? "Has location" : "No location", clear: { hasGps: null } });
  }
  if (q.hasText) chips.push({ key: "hasText", label: "Has text", clear: { hasText: false } });
  if (q.noText) chips.push({ key: "noText", label: "No text", clear: { noText: false } });
  for (const rule of q.inText ?? []) {
    const key = `inText:${rule.exact ? '"' : ""}${rule.text}`;
    chips.push({ key, label: rule.exact ? `Text: “${rule.text}”` : `Text: ${rule.text}`, clear: { inText: q.inText.filter((r) => r !== rule) } });
  }
  if (q.camera?.trim()) chips.push({ key: "camera", label: q.camera, clear: { camera: null } });
  if (q.lens?.trim()) chips.push({ key: "lens", label: q.lens, clear: { lens: null } });
  return chips;
}

/**
 * The filter row under the page header: a *Filters…* popover plus a removable chip for
 * every active filter (UX spec §6.2).
 *
 * Every picker in the popover is a Mantine `Select` with `withinPortal: false`. A portalled
 * dropdown lands on the body, *outside* the popover, so choosing an option counted as a click
 * outside and dismissed the whole sheet before the filter applied; rendered inside the popover
 * the list is part of it.
 */
export function FilterChips({
  query,
  onChange,
  placement = "inline",
}: {
  query: AssetQuery;
  onChange: (patch: Partial<AssetQuery>) => void;
  /** `dock` floats the control in the bottom-right corner of the surface it sits in. */
  placement?: "inline" | "dock";
}) {
  const { devices } = useDevices();
  const { settings } = useSettings();
  const [open, setOpen] = useState(false);
  const name = (id: number) => devices.find((d) => d.id === id)?.displayName ?? `Device ${id}`;
  const chips = chipsFor(query, name);
  const docked = placement === "dock";

  return (
    <div className="filter-chips" data-placement={placement}>
      <Popover
        opened={open}
        onChange={setOpen}
        position={docked ? "top-end" : "bottom-start"}
        shadow="lg"
        width={320}
        withinPortal
        transitionProps={{ transition: docked ? "pop-bottom-right" : "pop-top-left", duration: 160 }}
      >
        <Popover.Target>
          <Button
            size="xs"
            variant={chips.length ? "light" : "subtle"}
            leftSection={<IconAdjustments size={14} />}
            onClick={() => setOpen((v) => !v)}
          >
            Filters
          </Button>
        </Popover.Target>
        <Popover.Dropdown className="filter-chips__dropdown">
          {/* One picker for what kind of item: photos or videos (media type) or one of the
              special kinds. Picking one clears the other, so the two never contradict. */}
          <Select
            size="xs"
            comboboxProps={{ withinPortal: false }}
            label="Kind"
            data={[
              { value: "", label: "Anything" },
              { value: "m0", label: "Photos" },
              { value: "m1", label: "Videos" },
              ...(Object.keys(KIND_LABELS) as AssetKind[]).map((k) => ({ value: `k${k}`, label: KIND_LABELS[k] })),
            ]}
            value={query.kind ? `k${query.kind}` : query.mediaType != null ? `m${query.mediaType}` : ""}
            onChange={(v) => {
              if (v === null) return;
              onChange(
                !v
                  ? { mediaType: null, kind: null }
                  : v.startsWith("m")
                    ? { mediaType: Number(v.slice(1)), kind: null }
                    : { kind: v.slice(1) as AssetKind, mediaType: null },
              );
            }}
          />
          <Select
            size="xs"
            comboboxProps={{ withinPortal: false }}
            label="Device"
            data={[{ value: "", label: "Any device" }, ...devices.map((d) => ({ value: String(d.id), label: d.displayName }))]}
            value={query.deviceIds.length ? String(query.deviceIds[0]) : ""}
            onChange={(v) => v !== null && onChange({ deviceIds: v ? [Number(v)] : [] })}
          />
          <Select
            size="xs"
            comboboxProps={{ withinPortal: false }}
            label="Location"
            data={[
              { value: "", label: "Anywhere" },
              { value: "yes", label: "Has location" },
              { value: "no", label: "No location" },
            ]}
            value={query.hasGps == null ? "" : query.hasGps ? "yes" : "no"}
            onChange={(v) => v !== null && onChange({ hasGps: v === "" ? null : v === "yes" })}
          />
          {/* Round 13-3: only while text in photos is read. */}
          {readTextOn(settings) && (
            <Checkbox size="xs" label="Has text" checked={query.hasText} onChange={(e) => onChange({ hasText: e.currentTarget.checked })} />
          )}
          <div className="filter-chips__rating">
            <span>Rating at least</span>
            <StarRating value={query.minRating ?? 0} onChange={(r) => onChange({ minRating: r || null })} />
          </div>
          <TextInput
            size="xs"
            label="Camera"
            placeholder="ILCE-7M4"
            value={query.camera ?? ""}
            onChange={(e) => onChange({ camera: e.currentTarget.value || null })}
          />
          <TextInput
            size="xs"
            label="Lens"
            placeholder="FE 35mm"
            value={query.lens ?? ""}
            onChange={(e) => onChange({ lens: e.currentTarget.value || null })}
          />
        </Popover.Dropdown>
      </Popover>
      {chips.map((c) => (
        <button key={`${c.key}-${c.label}`} type="button" className="filter-chips__chip" onClick={() => onChange(c.clear)}>
          {c.label}
          <IconX size={12} aria-label={`Remove ${c.label}`} />
        </button>
      ))}
    </div>
  );
}
