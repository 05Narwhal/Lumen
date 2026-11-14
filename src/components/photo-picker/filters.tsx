import { Chip, Popover, TextInput, UnstyledButton } from "@mantine/core";
import { IconCheck, IconChevronUp, IconSearch } from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { useDevices } from "@/hooks/use-devices";

export interface PickerFilter {
  rawOnly: boolean;
  deviceIds: number[];
}

export const NO_FILTER: PickerFilter = { rawOnly: false, deviceIds: [] };

/** Compact controls in the picker's bottom bar: *RAW only* and a searchable device list. */
export function PickerFilters({ value, onChange }: { value: PickerFilter; onChange: (f: PickerFilter) => void }) {
  const { devices } = useDevices();
  const [search, setSearch] = useState("");
  const shown = useMemo(() => {
    const q = search.trim().toLowerCase();
    return devices.filter((d) => !d.hidden && d.itemCount > 0 && (!q || d.displayName.toLowerCase().includes(q)));
  }, [devices, search]);

  const toggle = (id: number) =>
    onChange({ ...value, deviceIds: value.deviceIds.includes(id) ? value.deviceIds.filter((d) => d !== id) : [...value.deviceIds, id] });

  const n = value.deviceIds.length;
  const label = n === 0 ? "All devices" : n === 1 ? (devices.find((d) => d.id === value.deviceIds[0])?.displayName ?? "1 device") : `${n} devices`;

  return (
    <div className="photo-picker__filters">
      <Chip size="xs" checked={value.rawOnly} onChange={(rawOnly) => onChange({ ...value, rawOnly })}>
        RAW only
      </Chip>
      <Popover position="top-start" width={260} shadow="md" withinPortal onClose={() => setSearch("")}>
        <Popover.Target>
          <UnstyledButton className="photo-picker__device-btn" data-active={n > 0} aria-label="Filter by device">
            {label} <IconChevronUp size={12} />
          </UnstyledButton>
        </Popover.Target>
        <Popover.Dropdown className="photo-picker__devices">
          <TextInput
            size="xs"
            placeholder="Search devices"
            leftSection={<IconSearch size={12} />}
            value={search}
            onChange={(e) => setSearch(e.currentTarget.value)}
            data-autofocus
          />
          <ul className="photo-picker__device-list" aria-label="Devices">
            {shown.map((d) => {
              const on = value.deviceIds.includes(d.id);
              return (
                <li key={d.id}>
                  <UnstyledButton className="photo-picker__device" data-on={on} onClick={() => toggle(d.id)} aria-pressed={on}>
                    <span className="photo-picker__device-check">{on && <IconCheck size={12} />}</span>
                    <span className="photo-picker__device-name">{d.displayName}</span>
                    <span className="photo-picker__device-count">{d.itemCount.toLocaleString()}</span>
                  </UnstyledButton>
                </li>
              );
            })}
            {shown.length === 0 && <li className="photo-picker__device-empty">No devices match.</li>}
          </ul>
          {n > 0 && (
            <UnstyledButton className="photo-picker__device-clear" onClick={() => onChange({ ...value, deviceIds: [] })}>
              Show all devices
            </UnstyledButton>
          )}
        </Popover.Dropdown>
      </Popover>
    </div>
  );
}
