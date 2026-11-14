import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, onTestFinished } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { PeopleRoute } from "@/routes/people";
import { PetsRoute } from "./gui";

const pets = [
  { id: 1, name: "Milo", species: "dog", detectionCount: 40, photoCount: 38, coverDetectionId: 11 },
  { id: 2, name: null, species: "cat", detectionCount: 12, photoCount: 12, coverDetectionId: 21 },
  { id: 3, name: null, species: "dog", detectionCount: 5, photoCount: 5, coverDetectionId: 31 },
];

function mount(path: string, extra: ReactNode = null) {
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/people" element={<PeopleRoute />} />
          <Route path="/pets/:id" element={<PetsRoute />} />
        </Routes>
        {extra}
      </MemoryRouter>
    </Providers>,
  );
}

const tile = { id: 42, x: 0, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 };

describe("PetsRoute (Round 4C)", () => {
  const props = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
  beforeEach(() => {
    for (const p of props) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_pets", () => pets);
    mockCommand("pet_suggestions", () => []);
  });

  it("lists pets, named first, with what each is", async () => {
    mount("/people?show=pets");
    const milo = await screen.findByRole("link", { name: /Milo/ });
    expect(milo).toHaveAttribute("href", "/pets/1");
    expect(screen.getByRole("link", { name: /Unnamed cat/ })).toHaveAttribute("href", "/pets/2");
    expect(screen.getByText("38 photos")).toBeInTheDocument();
  });

  it("says what Pets is when there are none", async () => {
    mockCommand("list_pets", () => []);
    mount("/people?show=pets");
    expect(await screen.findByText(/Pets appear here once smart features have looked at your photos/)).toBeInTheDocument();
  });

  it("names a pet in place", async () => {
    mockCommand("name_pet", () => null);
    mount("/pets/2");
    const field = await screen.findByRole("textbox", { name: "Name" });
    await userEvent.type(field, "Luna{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("name_pet", { pet: 2, name: "Luna" }));
  });

  it("merges with a pet of the same species", async () => {
    mockCommand("merge_pets", () => ({ from: {}, into: 1, detections: [], tookName: false, movedRefusals: [], fromSpecies: "dog" }));
    mount("/pets/3");
    await userEvent.click(await screen.findByRole("button", { name: "Merge with…" }));
    await userEvent.click(await screen.findByRole("option", { name: /Milo/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("merge_pets", { into: 1, from: 3 }));
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Merged into Milo");
  });

  it("lists its own species first and the other under a heading, and merges a dog into a cat (Round 9A)", async () => {
    mockCommand("merge_pets", () => ({ from: {}, into: 2, detections: [], tookName: false, movedRefusals: [], fromSpecies: "dog" }));
    mockCommand("undo_merge_pets", () => null);
    mount("/pets/1");
    await userEvent.click(await screen.findByRole("button", { name: "Merge with…" }));
    const options = await screen.findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([expect.stringMatching(/^Unnamed5 photos/), expect.stringMatching(/^Unnamed12 photos/)]);
    expect(screen.getByRole("heading", { name: "Cats" })).toBeInTheDocument();
    await userEvent.click(options[1]);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("merge_pets", { into: 2, from: 1 }));
    const t = toastStore.get().toasts.at(-1);
    expect(t?.message).toBe("Merged Milo into the unnamed cat");
    t?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_merge_pets", expect.anything()));
  });

  it("asks about doubtful photos and answers with Undo", async () => {
    mockCommand("pet_suggestions", () => [{ faceId: 77, assetId: 7, similarity: 0.5 }]);
    mockCommand("answer_pet_suggestion", () => ({ detection: 77, pet: null, pinned: false }));
    mount("/pets/1");
    expect(await screen.findByRole("heading", { name: "Is this Milo?" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Yes, this is Milo" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("answer_pet_suggestion", { detection: 77, pet: 1, yes: true }));
  });

  it("gives a pet card its menu, and Not a pet offers Undo (Round 8B)", async () => {
    const undo = { pet: { id: 1, name: "Milo", species: "dog", status: "listed", coverDetectionId: 11, createdAt: 1 }, detections: [11] };
    mockCommand("not_a_pet", () => undo);
    mockCommand("undo_not_a_pet", () => null);
    const { ContextMenuHost } = await import("@/components/context-menu");
    mount("/people?show=pets", <ContextMenuHost />);
    fireEvent.contextMenu((await screen.findByRole("link", { name: /Milo/ })).closest("li")!);
    expect(await screen.findByRole("menuitem", { name: "Open" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Merge with…" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Remove from Pets" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "Not a pet" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("not_a_pet", { pet: 1 }));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_not_a_pet", { undo }));
  });

  it("on a pet's page, a photo can become its main photo or leave it, with Undo (Round 8B)", async () => {
    const places = [{ detection: 111, pet: 1, pinned: false }];
    mockCommand("asset_pets", () => [{ id: 111, petId: 1, name: "Milo", species: "dog", x: 0.1, y: 0.1, w: 0.2, h: 0.2 }]);
    mockCommand("set_pet_cover", () => 11);
    mockCommand("move_pet_photos", () => places);
    mockCommand("restore_pet_places", () => null);
    mockCommand("get_layout", () => ({ totalHeight: 200, itemCount: 1, rowHeights: [200], sections: [] }));
    mockCommand("get_layout_rows", () => [{ index: 0, kind: "tiles", y: 0, height: 200, section: 0, title: null, tiles: [tile] }]);
    const rect = HTMLElement.prototype.getBoundingClientRect;
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 1000, height: 800, top: 0, left: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    onTestFinished(() => {
      HTMLElement.prototype.getBoundingClientRect = rect;
    });
    const { ContextMenuHost } = await import("@/components/context-menu");
    mount("/pets/1", <ContextMenuHost />);
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Item 42" }));
    await userEvent.click(await screen.findByText("Use as main photo of Milo"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_pet_cover", { pet: 1, detection: 111 }));
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Item 42" }));
    await userEvent.click(await screen.findByText("Remove from Milo"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_pet_photos", { pet: 1, assets: [42], to: null }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_pet_places", { places }));
  });
});
