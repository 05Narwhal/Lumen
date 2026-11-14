import { IconCamera, IconDeviceIpad, IconDeviceMobile } from "@tabler/icons-react";
import { DEVICE_KIND_CAMERA } from "@/hooks/use-devices";
import type { DeviceGroup } from "@/types/generated/DeviceGroup";

/** The icon of a device section: tablets by their group (Round 4H), else by kind. */
export const deviceIcon = (kind: number | undefined, group?: DeviceGroup) =>
  group === "ipad" || group === "tablet" ? IconDeviceIpad : kind === DEVICE_KIND_CAMERA ? IconCamera : IconDeviceMobile;

/** The icon of a connected iPhone or iPad, from its lockdown `ProductType` ("iPad13,4"). */
export const appleDeviceIcon = (productType: string | null | undefined) =>
  productType?.startsWith("iPad") ? IconDeviceIpad : IconDeviceMobile;
