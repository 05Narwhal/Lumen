import { IconCamera, IconDeviceIpad, IconDeviceMobile } from "@tabler/icons-react";
import { describe, expect, it } from "vitest";
import { DEVICE_KIND_CAMERA, DEVICE_KIND_PHONE } from "@/hooks/use-devices";
import { appleDeviceIcon, deviceIcon } from "./device-icons";

describe("deviceIcon", () => {
  it("draws iPads and other tablets as tablets", () => {
    expect(deviceIcon(DEVICE_KIND_PHONE, "ipad")).toBe(IconDeviceIpad);
    expect(deviceIcon(DEVICE_KIND_PHONE, "tablet")).toBe(IconDeviceIpad);
  });

  it("keeps phones and cameras as they were", () => {
    expect(deviceIcon(DEVICE_KIND_PHONE, "iphone")).toBe(IconDeviceMobile);
    expect(deviceIcon(DEVICE_KIND_CAMERA, "camera")).toBe(IconCamera);
    expect(deviceIcon(DEVICE_KIND_CAMERA)).toBe(IconCamera);
  });
});

describe("appleDeviceIcon", () => {
  it("reads the product type of a connected device", () => {
    expect(appleDeviceIcon("iPad13,4")).toBe(IconDeviceIpad);
    expect(appleDeviceIcon("iPhone15,2")).toBe(IconDeviceMobile);
    expect(appleDeviceIcon(null)).toBe(IconDeviceMobile);
  });
});
