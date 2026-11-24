import { describe, expect, it } from "vitest";
import { describeValue, editText, PROPOSAL_KINDS, valueFromText } from "./proposals";

describe("proposal values", () => {
  it("knows the seven kinds the host stores", () => {
    expect(PROPOSAL_KINDS).toEqual(["location", "date", "tags", "label", "person", "title", "caption"]);
  });

  it("describes each kind in words a person reads", () => {
    expect(describeValue({ kind: "location", value: { lat: 46.05, lon: 14.5 } })).toBe("46.0500, 14.5000");
    expect(describeValue({ kind: "tags", value: ["beach", "sea"] })).toBe("beach, sea");
    expect(describeValue({ kind: "label", value: "Green" })).toBe("Green");
    expect(describeValue({ kind: "person", value: { face: 7, name: "Ana" } })).toBe("Ana");
    expect(describeValue({ kind: "title", value: "Sunset" })).toBe("Sunset");
    expect(describeValue({ kind: "caption", value: "A long day" })).toBe("A long day");
  });

  it("shows a date as written, not moved to the viewer's time zone", () => {
    const text = describeValue({ kind: "date", value: "2021-05-04T23:30:00" });
    expect(text).toMatch(/2021/);
    expect(text).toMatch(/11:30|23:30/);
    expect(text).toMatch(/4/);
  });

  it("an edit starts from the proposed text and reads back as the same kind", () => {
    expect(editText({ kind: "tags", value: ["a", "b"] })).toBe("a, b");
    expect(editText({ kind: "location", value: { lat: 1.5, lon: -2 } })).toBe("1.5, -2");
    expect(editText({ kind: "date", value: "2021-05-04T10:00:00" })).toBe("2021-05-04T10:00:00");
    expect(valueFromText("tags", " a, b ,, A ")).toEqual({ kind: "tags", value: ["a", "b"] });
    expect(valueFromText("title", "  Dusk ")).toEqual({ kind: "title", value: "Dusk" });
    expect(valueFromText("label", "Blue")).toEqual({ kind: "label", value: "Blue" });
    expect(valueFromText("location", "46.05, 14.5")).toEqual({ kind: "location", value: { lat: 46.05, lon: 14.5 } });
    expect(valueFromText("date", "2021-05-04T10:00")).toEqual({ kind: "date", value: "2021-05-04T10:00:00" });
    expect(valueFromText("person", "Ana", { kind: "person", value: { face: 7, name: "x" } })).toEqual({ kind: "person", value: { face: 7, name: "Ana" } });
  });

  it("text that is not a value of that kind reads as nothing", () => {
    expect(valueFromText("title", "   ")).toBeNull();
    expect(valueFromText("tags", " , ")).toBeNull();
    expect(valueFromText("location", "north")).toBeNull();
    expect(valueFromText("location", "95, 0")).toBeNull();
    expect(valueFromText("date", "yesterday")).toBeNull();
    expect(valueFromText("person", "Ana")).toBeNull();
  });
});
