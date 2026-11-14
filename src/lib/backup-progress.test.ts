import { describe, expect, it } from "vitest";
import { backupFinishedMessage, backupProgressJob } from "./backup-progress";

describe("backup progress", () => {
  it("puts a running backup in the activity ring by bytes", () => {
    const job = backupProgressJob({ doneBytes: 25e9, totalBytes: 100e9, doneFiles: 400, totalFiles: 1600 });
    expect(job).toMatchObject({ id: "backup", kind: "backup", state: "running", progress: 0.25, label: "Backing up" });
    expect(job.detail).toBe("400 of 1,600 files");
    // Nothing to copy is a finished backup, not a division by zero.
    expect(backupProgressJob({ doneBytes: 0, totalBytes: 0, doneFiles: 0, totalFiles: 0 }).progress).toBe(1);
  });

  it("says how a backup ended in a sentence", () => {
    const report = { copied: 3, copiedBytes: 2_500_000, unchanged: 10, retired: 1, failed: [], cancelled: false };
    expect(backupFinishedMessage({ at: 1, error: null, report })).toBe("Backed up 3 new files (2.4 MB)");
    expect(backupFinishedMessage({ at: 1, error: null, report: { ...report, copied: 0 } })).toBe("Backup is up to date");
    expect(backupFinishedMessage({ at: 1, error: null, report: { ...report, failed: ["a", "b"] } })).toBe("Backed up 3 new files (2.4 MB); 2 could not be copied and will be tried again");
    expect(backupFinishedMessage({ at: 1, error: null, report: { ...report, cancelled: true } })).toBe("Backup stopped; it picks up from here next time");
    expect(backupFinishedMessage({ at: 1, error: "The backup drive is not there.", report: null })).toBe("The backup drive is not there.");
  });
});
