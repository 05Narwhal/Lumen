import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { LibraryDetail } from "@/types";
import { DeleteLibraries } from "./gui";

const GB = 1024 ** 3;
const MAIN: LibraryDetail = { name: "Lumen Library", path: "C:\\Users\\A\\Pictures\\Lumen Library", photos: 5012, bytes: Math.round(38.4 * GB), refused: null };
const SCANS: LibraryDetail = { name: "Family scans", path: "D:\\Photos\\Family scans", photos: 812, bytes: Math.round(6.6 * GB), refused: null };
const ODD: LibraryDetail = { name: "Old phone", path: "C:\\AppData\\Lumen\\Old phone", photos: 0, bytes: 0, refused: "holds-lumen" };

function setup(libraries: LibraryDetail[] | null, extra: Partial<Parameters<typeof DeleteLibraries>[0]> = {}) {
  const props = { libraries, checking: false, tooBig: null, onConfirm: vi.fn(), onEraseAnyway: vi.fn(), onCancelTooBig: vi.fn(), onBack: vi.fn(), ...extra };
  render(<DeleteLibraries {...props} />);
  return props;
}

describe("DeleteLibraries", () => {
  it("says it is counting while the details load", () => {
    setup(null);
    expect(screen.getByRole("status")).toHaveTextContent("Counting photos…");
  });

  it("with one library, deletes only once its exact name is typed", async () => {
    const p = setup([MAIN]);
    expect(screen.getByRole("heading")).toHaveTextContent("Delete Lumen Library?");
    expect(screen.getByText("5,012 photos · 38 GB")).toBeInTheDocument();
    const go = screen.getByRole("button", { name: "Delete library" });
    const field = screen.getByLabelText('Type "Lumen Library" to delete it');
    expect(go).toBeDisabled();
    await userEvent.type(field, "lumen library");
    expect(go).toBeDisabled();
    await userEvent.clear(field);
    await userEvent.type(field, "Lumen Library");
    expect(go).toBeEnabled();
    await userEvent.click(go);
    expect(p.onConfirm).toHaveBeenCalledWith([MAIN.path]);
  });

  it("with several, starts with none ticked and needs a choice and DELETE", async () => {
    const p = setup([MAIN, SCANS, ODD]);
    const go = screen.getByRole("button", { name: /^Delete/ });
    const field = screen.getByLabelText("Type DELETE to delete them");
    expect(screen.getByRole("checkbox", { name: /Lumen Library/ })).not.toBeChecked();
    await userEvent.type(field, "DELETE");
    expect(go).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /Family scans/ }));
    expect(go).toBeEnabled();
    expect(go).toHaveTextContent("Delete 1 library");
    await userEvent.click(go);
    expect(p.onConfirm).toHaveBeenCalledWith([SCANS.path]);
  });

  it("needs DELETE exactly, in capitals", async () => {
    setup([MAIN, SCANS]);
    await userEvent.click(screen.getByRole("button", { name: "Select all" }));
    await userEvent.type(screen.getByLabelText("Type DELETE to delete them"), "delete");
    expect(screen.getByRole("button", { name: "Delete 2 libraries" })).toBeDisabled();
  });

  it("select all ticks every library it may delete and never a refused one", async () => {
    setup([MAIN, SCANS, ODD]);
    await userEvent.click(screen.getByRole("button", { name: "Select all" }));
    expect(screen.getByRole("checkbox", { name: /Lumen Library/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /Family scans/ })).toBeChecked();
    expect(screen.queryByRole("checkbox", { name: /Old phone/ })).not.toBeInTheDocument();
    expect(screen.getByText("Lumen's own files are inside this library. Delete it yourself after removing Lumen.")).toBeInTheDocument();
    expect(screen.getByText("2 of 2 · 45 GB")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Select none" }));
    expect(screen.getByRole("checkbox", { name: /Lumen Library/ })).not.toBeChecked();
  });

  it("when they don't fit the Recycle Bin, says so and asks", async () => {
    const p = setup([MAIN], { tooBig: { fits: false, bytes: Math.round(38.4 * GB), room: 4 * GB } });
    expect(screen.getByRole("alert")).toHaveTextContent("This library is too big for the Recycle Bin (38 GB; it has room for 4.0 GB). It would be deleted for good.");
    await userEvent.click(screen.getByRole("button", { name: "Delete for good" }));
    expect(p.onEraseAnyway).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(p.onCancelTooBig).toHaveBeenCalledOnce();
  });

  it("says when a drive has no Recycle Bin at all", () => {
    setup([MAIN, SCANS], { tooBig: { fits: false, bytes: 1, room: 0 } });
    expect(screen.getByRole("alert")).toHaveTextContent("These libraries can't go to the Recycle Bin on this drive. They would be deleted for good.");
  });

  it("offers only Back when nothing can be deleted here", async () => {
    const p = setup([ODD]);
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(p.onBack).toHaveBeenCalledOnce();
  });
});
