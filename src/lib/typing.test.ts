import { describe, expect, it } from "vitest";
import { isTyping } from "./typing";

describe("isTyping (Round 8B)", () => {
  const input = (type: string) => Object.assign(document.createElement("input"), { type });

  it("is true in a field that takes text", () => {
    expect(isTyping(input("text"))).toBe(true);
    expect(isTyping(input("search"))).toBe(true);
    expect(isTyping(input("number"))).toBe(true);
    expect(isTyping(document.createElement("textarea"))).toBe(true);
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    expect(isTyping(editable)).toBe(true);
  });

  it("is false on a slider, a checkbox, a button or nothing, so their keys still reach the page", () => {
    expect(isTyping(input("range"))).toBe(false);
    expect(isTyping(input("checkbox"))).toBe(false);
    expect(isTyping(document.createElement("button"))).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});
