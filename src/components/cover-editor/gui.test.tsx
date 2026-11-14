import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { photoItems } from "@/components/cover-stack";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { CoverEditor } from "./gui";
import type { CoverTarget } from "./store";

const album: CoverTarget = { kind: "album", id: 7, name: "Lisbon", style: null, photos: photoItems([1, 2, 3]) };

function mount(target: CoverTarget, onClose = vi.fn()) {
  render(
    <Providers>
      <CoverEditor target={target} onClose={onClose} />
    </Providers>,
  );
  return onClose;
}

// Julian's 2026-11-08 list, item 3: preset swatches and a picker of one's own; a shape and an icon.
describe("CoverEditor", () => {
  it("turns an album's photos into a shape with an icon in a swatch, with Undo", async () => {
    mockCommand("set_album_cover_style", () => null);
    const onClose = mount(album);
    expect(document.querySelectorAll(".cover-editor__preview .cover-stack__print")).toHaveLength(3);
    await userEvent.click(screen.getByText("Icon"));
    await userEvent.click(screen.getByRole("button", { name: "Box" }));
    await userEvent.click(screen.getByRole("button", { name: "Travel" }));
    await userEvent.click(screen.getByRole("button", { name: "Colour 3" }));
    expect(screen.getByRole("button", { name: "Colour 3" })).toHaveAttribute("aria-pressed", "true");
    expect(document.querySelector(".cover-editor__preview .icon-cover")).toHaveAttribute("data-shape", "box");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onClose).toHaveBeenCalled();
    const style = { kind: "icon", shape: "box", icon: "plane", color: "swatch-3" };
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_album_cover_style", { albumId: 7, style }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Cover changed"));
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenLastCalledWith("set_album_cover_style", { albumId: 7, style: null }));
  });

  it("offers a colour of one's own and saves a collection's photos back", async () => {
    mockCommand("set_collection_cover_style", () => null);
    mount({ kind: "collection", id: 3, name: "Trips", style: { kind: "icon", shape: "folder", icon: "map-pin", color: "swatch-1" }, photos: [] });
    await userEvent.click(screen.getByRole("button", { name: "Your own colour" }));
    expect(screen.getByRole("button", { name: "Your own colour" })).toHaveAttribute("aria-pressed", "true");
    expect(document.querySelector(".mantine-ColorPicker-wrapper, .cover-editor__picker")).not.toBeNull();
    await userEvent.click(screen.getByText("Photos"));
    expect(screen.getByText("No photos yet")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_collection_cover_style", { id: 3, style: null }));
  });
});
