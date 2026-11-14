import { describe, expect, it } from "vitest";
import { GROUP_LABELS, groupDevices } from "./device-groups";

const d = (id: number, group: "iphone" | "ipad" | "camera" | "other") => ({ id, name: `Device ${id}`, group });

describe("groupDevices", () => {
  it("gathers devices of one kind, in the order their newest import put them", () => {
    const rows = groupDevices([d(1, "iphone"), d(2, "camera"), d(3, "iphone"), d(4, "ipad")]);
    expect(rows).toEqual([
      { kind: "group", group: "iphone", label: "iPhones", devices: [d(1, "iphone"), d(3, "iphone")] },
      { kind: "device", device: d(2, "camera") },
      { kind: "device", device: d(4, "ipad") },
    ]);
  });

  it("treats a device without a group as other", () => {
    expect(groupDevices([{ id: 9, name: "Scanner" }])).toEqual([{ kind: "device", device: { id: 9, name: "Scanner" } }]);
  });

  it("names every group", () => {
    expect(GROUP_LABELS.camera).toBe("Cameras");
    expect(GROUP_LABELS.actionCamera).toBe("Action cameras");
  });
});
