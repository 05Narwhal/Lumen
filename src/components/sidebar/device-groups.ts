import type { DeviceGroup } from "@/types/generated/DeviceGroup";

export const GROUP_LABELS: Record<DeviceGroup, string> = {
  iphone: "iPhones",
  ipad: "iPads",
  tablet: "Tablets",
  phone: "Phones",
  camera: "Cameras",
  drone: "Drones",
  actionCamera: "Action cameras",
  computer: "Computers",
  other: "Other devices",
};

export interface GroupableDevice {
  id: number;
  name: string;
  group?: DeviceGroup;
}

export type DeviceRow<D extends GroupableDevice> =
  | { kind: "group"; group: DeviceGroup; label: string; devices: D[] }
  | { kind: "device"; device: D };

/**
 * The sidebar's device rows: devices of one kind gathered under one row, a kind with a
 * single device shown as that device. Rows keep the order of each kind's first device,
 * which is the order of their newest import.
 */
export function groupDevices<D extends GroupableDevice>(devices: D[]): DeviceRow<D>[] {
  const byGroup = new Map<DeviceGroup, D[]>();
  for (const d of devices) {
    const g = d.group ?? "other";
    const list = byGroup.get(g);
    if (list) list.push(d);
    else byGroup.set(g, [d]);
  }
  return [...byGroup].map(([group, list]) =>
    list.length === 1 ? { kind: "device", device: list[0] } : { kind: "group", group, label: GROUP_LABELS[group], devices: list },
  );
}
