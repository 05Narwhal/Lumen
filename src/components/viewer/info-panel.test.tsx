import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { toastStore } from "@/components/toast/store";
import { useFaceTags } from "@/hooks/use-people";
import { usePetBoxes } from "@/hooks/use-pets";
import { ContextMenuHost } from "@/components/context-menu";
import type { AssetFace } from "@/types/generated/AssetFace";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import { InfoPanel, type FaceTools } from "./info-panel";

const asset = {
  id: 5,
  filename: "IMG_5.jpg",
  extension: "jpg",
  mediaType: 0,
  byteSize: 1000,
  width: 4000,
  height: 3000,
  orientation: 1,
  capturedAt: 1_758_000_000,
  importedAt: 1_758_000_000,
  durationMs: null,
  deviceName: null,
  gpsLat: null,
  gpsLon: null,
  locality: null,
  placeName: null,
  placeSource: 0,
  isFavorite: false,
  rating: 0,
  hdr: 0,
  relativePath: "5.jpg",
  contentHash: "abc",
  metadata: {},
  renderable: true,
  raw: false,
  liveVideoId: null,
  colorLabel: 0,
  flag: 0,
  derivedFrom: null,
} as unknown as AssetDetail;

describe("InfoPanel", () => {
  it("makes a face the main photo of the person it is (Round 4B)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("faces_for_asset", () => [{ id: 31, personId: 1, name: "Anna", x: 0.1, y: 0.1, w: 0.1, h: 0.1 }]);
    mockCommand("set_person_cover", () => 10);
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Use as main photo of Anna" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_cover", { person: 1, face: 31 }));
  });

  const faces: AssetFace[] = [
    { id: 31, personId: 1, name: "Anna", x: 0.1, y: 0.1, w: 0.1, h: 0.1, manual: false },
    { id: 32, personId: null, name: null, x: 0.5, y: 0.1, w: 0.1, h: 0.1, manual: false },
  ];
  const tools = (): FaceTools => ({ show: false, onToggleShow: vi.fn(), drawing: false, onAddFace: vi.fn(), onTag: vi.fn(), onUntag: vi.fn(), onHover: vi.fn() });

  it("lists every face, Unknown with Tag… and a named one with Not {name} (Round 4I)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("faces_for_asset", () => faces);
    const t = tools();
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} faceTools={t} />
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Tag…" }));
    expect(t.onTag).toHaveBeenCalledWith(faces[1]);
    await userEvent.click(screen.getByRole("button", { name: "Not Anna" }));
    expect(t.onUntag).toHaveBeenCalledWith(faces[0]);
    await userEvent.hover(screen.getByText("Unknown"));
    expect(t.onHover).toHaveBeenCalledWith(32);
    await userEvent.click(screen.getByRole("button", { name: /Show faces/ }));
    expect(t.onToggleShow).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: /Add a face/ }));
    expect(t.onAddFace).toHaveBeenCalled();
  });

  it("links a named pet to its page (Round 4C)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("asset_pets", () => [{ id: 41, petId: 1, name: "Milo", species: "dog", x: 0.1, y: 0.1, w: 0.3, h: 0.3 }]);
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByRole("link", { name: "Milo" })).toHaveAttribute("href", "/pets/1");
  });

  it("tags an unknown pet as one of its species, with Undo (Round 4C)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("asset_pets", () => [{ id: 42, petId: null, name: null, species: "cat", x: 0.1, y: 0.1, w: 0.3, h: 0.3 }]);
    mockCommand("list_pets", () => [
      { id: 1, name: "Milo", species: "dog", detectionCount: 4, photoCount: 4, coverDetectionId: 11 },
      { id: 2, name: "Luna", species: "cat", detectionCount: 3, photoCount: 3, coverDetectionId: 21 },
    ]);
    const place = { detection: 42, pet: null, pinned: false };
    mockCommand("tag_pet", () => place);
    mockCommand("restore_pet_places", () => null);
    toastStore.set({ toasts: [] });
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Unknown cat")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "Tag…" }));
    expect(screen.queryByText("Milo")).not.toBeInTheDocument();
    await userEvent.click(await screen.findByText("Luna"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("tag_pet", { detection: 42, pet: 2 }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Tagged as Luna"));
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_pet_places", { places: [place] }));
  });

  it("gives a pet row the pet box's menu: Open {name}, and Not {name} with Undo (Round 8B)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("asset_pets", () => [{ id: 41, petId: 1, name: "Milo", species: "dog", x: 0.1, y: 0.1, w: 0.3, h: 0.3 }]);
    mockCommand("list_pets", () => [{ id: 1, name: "Milo", species: "dog", detectionCount: 4, photoCount: 4, coverDetectionId: 11 }]);
    const place = { detection: 41, pet: 1, pinned: false };
    mockCommand("untag_pet_detection", () => place);
    mockCommand("restore_pet_places", () => null);
    toastStore.set({ toasts: [] });
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    fireEvent.contextMenu((await screen.findByRole("link", { name: "Milo" })).closest("li")!);
    expect(await screen.findByRole("menuitem", { name: "Open Milo" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Use as main photo of Milo" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "Not Milo" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("untag_pet_detection", { detection: 41 }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_pet_places", { places: [place] }));
  });

  it("shows the stored place, else the coordinates, never a live atlas guess (exact place names)", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const at = { gpsLat: 46.0006, gpsLon: 14.0264 };
    const show = (a: Partial<AssetDetail>) =>
      render(
        <Providers>
          <MemoryRouter>
            <InfoPanel asset={{ ...asset, ...at, ...a }} onReveal={() => {}} />
          </MemoryRouter>
        </Providers>,
      );
    const online = show({ locality: "Jelični Vrh, Slovenia", placeName: "Idrija, Slovenia", placeSource: 1 });
    expect(await screen.findByText("Jelični Vrh, Slovenia")).toBeInTheDocument();
    expect(screen.getByText("© OpenStreetMap contributors")).toBeInTheDocument();
    online.unmount();
    const atlas = show({ placeName: "Gorizia, Italy" });
    expect(await screen.findByText("Gorizia, Italy")).toBeInTheDocument();
    expect(screen.queryByText("© OpenStreetMap contributors")).not.toBeInTheDocument();
    atlas.unmount();
    show({});
    expect(await screen.findByText("46.00060, 14.02640")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("place_for", expect.anything());
  });

  it("offers Add a face on a photo where no one was found", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    render(
      <Providers>
        <MemoryRouter>
          <InfoPanel asset={asset} onReveal={() => {}} faceTools={tools()} />
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("No one found in this photo.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Add a face/ })).toBeInTheDocument();
  });
});

describe("useFaceTags", () => {
  const wrapper = ({ children }: { children: React.ReactNode }) => <Providers>{children}</Providers>;

  it("tags a face, and Undo puts it back where it was", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const place = { face: 32, person: null, pinned: false };
    mockCommand("tag_face", () => place);
    mockCommand("restore_faces", () => null);
    toastStore.set({ toasts: [] });
    const { result } = renderHook(() => useFaceTags(), { wrapper });
    await result.current.tag({ id: 32, personId: null, name: null, x: 0, y: 0, w: 0.1, h: 0.1, manual: false }, { person: { id: 1, name: "Anna", faceCount: 3, photoCount: 3, coverFaceId: null } });
    expect(invokeMock).toHaveBeenCalledWith("tag_face", { face: 32, person: 1 });
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("Tagged Anna");
    t.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_faces", { places: [place] }));
  });

  it("makes a new person from a face", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("create_person_with_face", () => [9, { face: 32, person: null, pinned: false }]);
    const { result } = renderHook(() => useFaceTags(), { wrapper });
    await result.current.tag({ id: 32, personId: null, name: null, x: 0, y: 0, w: 0.1, h: 0.1, manual: false }, { newName: " Zoe " });
    expect(invokeMock).toHaveBeenCalledWith("create_person_with_face", { face: 32, name: "Zoe" });
  });

  it("tags a drawn box on a new person, and Undo removes both", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("add_face_box", () => [77, 9]);
    mockCommand("untag_face", () => ({ place: { face: 77, person: 9, pinned: true }, manual: null }));
    mockCommand("undo_create_person", () => null);
    toastStore.set({ toasts: [] });
    const { result } = renderHook(() => useFaceTags(), { wrapper });
    await result.current.addBox(5, [0.1, 0.2, 0.3, 0.4], { newName: "Zoe" });
    expect(invokeMock).toHaveBeenCalledWith("add_face_box", { assetId: 5, rect: [0.1, 0.2, 0.3, 0.4], personId: null, newName: "Zoe" });
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_create_person", { person: 9, place: { face: 77, person: null, pinned: false } }));
    expect(invokeMock).toHaveBeenCalledWith("untag_face", { face: 77 });
  });
});

describe("usePetBoxes (Round 8B)", () => {
  const wrapper = ({ children }: { children: React.ReactNode }) => <Providers>{children}</Providers>;
  const box = { id: 41, petId: 1, name: "Milo", species: "dog", x: 0.1, y: 0.1, w: 0.3, h: 0.3 };

  it("says Not a pet about one box, and Undo makes it a pet again", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const place = { detection: 41, pet: 1, pinned: false };
    mockCommand("reject_pet_detection", () => place);
    mockCommand("undo_reject_pet_detection", () => null);
    toastStore.set({ toasts: [] });
    const { result } = renderHook(() => usePetBoxes(), { wrapper });
    await result.current.reject(box);
    expect(invokeMock).toHaveBeenCalledWith("reject_pet_detection", { detection: 41 });
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_reject_pet_detection", { place }));
  });

  it("makes a new pet from one box, and Undo removes it", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    const place = { detection: 41, pet: 1, pinned: false };
    mockCommand("create_pet_with_detection", () => [9, place]);
    mockCommand("undo_create_pet", () => null);
    toastStore.set({ toasts: [] });
    const { result } = renderHook(() => usePetBoxes(), { wrapper });
    await result.current.create(box, " Rex ");
    expect(invokeMock).toHaveBeenCalledWith("create_pet_with_detection", { detection: 41, name: "Rex" });
    expect(toastStore.get().toasts.at(-1)!.message).toBe("New pet: Rex");
    toastStore.get().toasts.at(-1)!.action!.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_create_pet", { pet: 9, place }));
  });
});
