import { describe, expect, it } from "vitest";
import { splitDrop } from "./gui";

describe("splitDrop", () => {
  it("imports dropped files as themselves and dropped folders whole", () => {
    expect(splitDrop(["C:\\Pics\\a.JPG", "C:\\Pics\\b.cr3", "D:\\Trip", "/Users/j/Photos/x.mov", "D:\\Trip"])).toEqual({
      files: ["C:\\Pics\\a.JPG", "C:\\Pics\\b.cr3", "/Users/j/Photos/x.mov"],
      folders: ["D:\\Trip"],
    });
  });

  it("treats a folder whose name has a dot as a folder", () => {
    expect(splitDrop(["D:\\Photos 2024.backup", "E:\\DCIM\\100CANON"])).toEqual({ files: [], folders: ["D:\\Photos 2024.backup", "E:\\DCIM\\100CANON"] });
  });
});
