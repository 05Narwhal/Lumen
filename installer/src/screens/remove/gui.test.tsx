import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectEveryControlReachableByTab } from "@/test/tab";
import { Remove } from "./index";

const GB = 1024 ** 3;
const base = {
  libraries: ["C:\\Users\\Sample\\Pictures\\Lumen Library", "D:\\Photos\\Second"],
  dataBytes: Math.round(1.9 * GB),
  safetyCopies: null as string | null,
  removeData: false,
  phase: "confirm" as "confirm" | "removing" | "done",
  onToggleData: vi.fn(),
  onRemove: vi.fn(),
  onCancel: vi.fn(),
  onFinish: vi.fn(),
};
const KEPT = "C:\\AppData\\backups\\pre-migration";

describe("Remove: deleting libraries", () => {
  it("offers Delete libraries… as its own step, deleting nothing by itself", async () => {
    const onDeleteLibraries = vi.fn();
    render(<Remove {...base} onDeleteLibraries={onDeleteLibraries} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete libraries…" }));
    expect(onDeleteLibraries).toHaveBeenCalledOnce();
    expect(base.onRemove).not.toHaveBeenCalled();
  });

  it("says which libraries will go and how, with Undo, and stops promising they stay", async () => {
    const onUndoDelete = vi.fn();
    render(<Remove {...base} deleteChoice={{ paths: [base.libraries[0]], names: ["Lumen Library"], mode: "recycle" }} onUndoDelete={onUndoDelete} onDeleteLibraries={vi.fn()} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Lumen Library will be moved to the Recycle Bin.");
    expect(screen.getByText("Your other libraries are not touched.")).toBeInTheDocument();
    expect(screen.queryByText("C:\\Users\\Sample\\Pictures\\Lumen Library")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Delete libraries…" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(onUndoDelete).toHaveBeenCalledOnce();
  });

  it("says when they will be deleted for good", () => {
    render(<Remove {...base} deleteChoice={{ paths: base.libraries, names: ["Lumen Library", "Second"], mode: "erase" }} onUndoDelete={vi.fn()} />);
    expect(screen.getByRole("alert")).toHaveTextContent("2 libraries will be deleted for good: Lumen Library, Second.");
  });

  it("ends saying what went and what could not", () => {
    render(
      <Remove
        {...base}
        phase="done"
        deleted={[
          { path: base.libraries[0], folderRemoved: true, problem: null },
          { path: base.libraries[1], folderRemoved: false, problem: "Some of this library could not be deleted (in use). What is left is still in its folder." },
        ]}
        deleteChoice={{ paths: base.libraries, names: ["Lumen Library", "Second"], mode: "recycle" }}
      />,
    );
    expect(screen.getByText("Lumen Library was moved to the Recycle Bin.")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Second: Some of this library could not be deleted");
    expect(screen.queryByText("Your photos and libraries were not touched.")).not.toBeInTheDocument();
  });
});

describe("Remove", () => {
  it("lists every library as untouched and leaves the data box off", () => {
    render(<Remove {...base} />);
    expect(screen.getByText("Your photos and libraries are not touched.")).toBeInTheDocument();
    expect(screen.getByText("C:\\Users\\Sample\\Pictures\\Lumen Library")).toBeInTheDocument();
    expect(screen.getByText("D:\\Photos\\Second")).toBeInTheDocument();
    const box = screen.getByRole("checkbox", { name: /Also remove Lumen's settings, caches and downloaded models \(1\.9 GB\)/ });
    expect(box).not.toBeChecked();
  });

  it("reports the box and the two actions", async () => {
    const onToggleData = vi.fn();
    const onRemove = vi.fn();
    const onCancel = vi.fn();
    render(<Remove {...base} onToggleData={onToggleData} onRemove={onRemove} onCancel={onCancel} />);
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Remove Lumen" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onToggleData).toHaveBeenCalledWith(true);
    expect(onRemove).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("says where the safety copies are kept", () => {
    render(<Remove {...base} safetyCopies={KEPT} />);
    const path = screen.getByText(KEPT);
    expect(path).toHaveClass("s-path");
    expect(path.parentElement).toHaveTextContent(`Safety copies of your catalogues are kept in ${KEPT}`);
  });

  it("still promises the libraries are untouched when none is registered", () => {
    render(<Remove {...base} libraries={[]} />);
    expect(screen.getByText("Your photos and libraries are not touched.")).toBeInTheDocument();
  });

  it("shows a short progress while removing, without buttons that change anything", () => {
    render(<Remove {...base} phase="removing" />);
    expect(screen.getByText("Removing Lumen…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Remove Lumen" })).toBeNull();
  });

  it("waits on Lumen while removing, with the import line and Cancel", async () => {
    const onCancel = vi.fn();
    render(<Remove {...base} phase="removing" closing="asking-import" onCancel={onCancel} />);
    expect(screen.getByRole("status")).toHaveTextContent("Lumen is asking about the import that's running.");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("names what went wrong and keeps Remove Lumen", () => {
    render(<Remove {...base} error="Lumen's uninstaller stopped with code 2. Lumen is still installed; try again." />);
    expect(screen.getByRole("alert")).toHaveTextContent("Lumen is still installed");
    expect(screen.getByRole("button", { name: "Remove Lumen" })).toBeEnabled();
  });

  it("ends with Lumen is removed, naming the safety copies when they stay", async () => {
    const onFinish = vi.fn();
    render(<Remove {...base} phase="done" safetyCopies={KEPT} onFinish={onFinish} />);
    // A long path never sits in the heading (it ran off the window).
    expect(screen.getByRole("heading")).toHaveTextContent(/^Lumen is removed\.$/);
    expect(screen.getByText("Its safety copies stay in")).toBeInTheDocument();
    expect(screen.getByText(KEPT)).toHaveClass("s-path");
    await userEvent.click(screen.getByRole("button", { name: "Finish" }));
    expect(onFinish).toHaveBeenCalledOnce();
  });

  it("ends with a plain Lumen is removed when nothing is kept", () => {
    render(<Remove {...base} phase="done" />);
    expect(screen.getByText("Lumen is removed.")).toBeInTheDocument();
  });

  it("every_control_is_reachable_by_tab", async () => {
    const { container } = render(<Remove {...base} />);
    await expectEveryControlReachableByTab(container);
  });
});
