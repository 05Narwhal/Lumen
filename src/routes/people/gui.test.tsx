import type React from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, onTestFinished } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { PeopleRoute } from "./gui";

const people = [
  { id: 1, name: "Anna", faceCount: 40, photoCount: 31, coverFaceId: 10 },
  { id: 2, name: null, faceCount: 12, photoCount: 9, coverFaceId: 20 },
];

function mount(path = "/people", extra: React.ReactNode = null, setup = () => {}) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  mockCommand("list_people", () => people);
  setup();
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/people" element={<PeopleRoute />} />
          <Route path="/people/:id" element={<PeopleRoute />} />
        </Routes>
        {extra}
      </MemoryRouter>
    </Providers>,
  );
}

/** Drags `from` onto `to` with the pointer, as the People page listens for it. */
function dragOnto(from: Element, to: Element) {
  const hit = document.elementFromPoint;
  document.elementFromPoint = () => to;
  fireEvent.pointerDown(from, { button: 0, clientX: 10, clientY: 10 });
  fireEvent.pointerMove(window, { clientX: 40, clientY: 40 });
  fireEvent.pointerMove(window, { clientX: 60, clientY: 60 });
  fireEvent.pointerUp(window, { clientX: 60, clientY: 60 });
  document.elementFromPoint = hit;
}

const SIZES = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;

describe("People", () => {
  // jsdom lays nothing out; the grid needs a width before it asks Rust for a layout.
  beforeEach(() => {
    for (const p of SIZES) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
    toastStore.set({ toasts: [] });
    mockCommand("list_person_groups", () => []);
  });
  afterEach(() => {
    for (const p of SIZES) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  // Round 20: People & Pets is one page; the tab is in the URL.
  it("draws People and Pets with their counts, and one body alone on its tab", async () => {
    mount("/people", null, () => mockCommand("list_pets", () => [{ id: 1, name: "Milo", species: "dog", detectionCount: 4, photoCount: 4, coverDetectionId: 11 }]));
    expect(await screen.findByRole("heading", { level: 1, name: "People & Pets" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { level: 2, name: /People\s*2/ })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { level: 2, name: /Pets\s*1/ })).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: /Milo/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: "People" }));
    await waitFor(() => expect(screen.queryByRole("link", { name: /Milo/ })).toBeNull());
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    expect(screen.getByRole("radio", { name: "People" })).toHaveAttribute("aria-checked", "true");
  });

  // Round 15: while groups hang or fail, nobody is drawn ungrouped.
  it("says People could not load when the groups fail, and Try again loads them", async () => {
    let fail = true;
    mockCommand("list_person_groups", () => {
      if (fail) throw { code: "db", message: "busy" };
      return [];
    });
    mount();
    await screen.findByText("People could not load.");
    expect(screen.queryByText("Anna")).not.toBeInTheDocument();
    fail = false;
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Anna");
  });

  it("draws no person card while the groups are still loading", async () => {
    mockCommand("list_person_groups", () => new Promise(() => {}));
    mount();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_people", undefined));
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText("Anna")).not.toBeInTheDocument();
    expect(screen.queryByText("No people yet")).not.toBeInTheDocument();
  });

  it("lists named people first, and asks for a name for the rest", async () => {
    mount();
    await screen.findByText("Anna");
    const cards = screen.getAllByRole("listitem").filter((li) => !li.textContent?.includes("New group"));
    expect(cards[0]).toHaveTextContent("Anna");
    expect(cards[0]).toHaveTextContent("31 photos");
    expect(cards[1]).toHaveTextContent("Add a name");
  });

  it("merges a person dropped on another, and offers Undo", async () => {
    const undo = { faces: [5, 6], name: null, tookName: false, into: 1 };
    mockCommand("merge_people", () => undo);
    mockCommand("restore_person", () => 3);
    mount();
    const anna = (await screen.findByText("Anna")).closest("li")!;
    const other = screen.getByText("Add a name").closest("li")!;
    dragOnto(other, anna);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("merge_people", { into: 1, from: 2 }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_person", { undo }));
  });

  it("shows groups first as circles of up to three faces, and only ungrouped people after them", async () => {
    mockCommand("list_person_groups", () => [{ id: 7, name: "Family", personIds: [1] }]);
    mount();
    const family = (await screen.findByText("Family")).closest("li")!;
    const items = screen.getAllByRole("listitem");
    expect(items[0]).toBe(family);
    expect(family).toHaveTextContent("1 person");
    expect(family.querySelectorAll(".people-route__stack-face")).toHaveLength(1);
    expect(items.some((li) => li !== family && li.textContent?.includes("Anna"))).toBe(false);
    expect(screen.getByText("Add a name")).toBeInTheDocument();
  });

  it("adds a person dropped on a group", async () => {
    mockCommand("list_person_groups", () => [{ id: 7, name: "Family", personIds: [] }]);
    mockCommand("set_person_group_members", () => null);
    mount();
    const family = (await screen.findByText("Family")).closest("li")!;
    dragOnto(screen.getByText("Anna").closest("li")!, family);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_group_members", { group: 7, people: [1], member: true }));
    expect(invokeMock).not.toHaveBeenCalledWith("merge_people", expect.anything());
  });

  it("starts a group with a person dropped on New group", async () => {
    mockCommand("list_person_groups", () => []);
    mockCommand("create_person_group", () => 9);
    mockCommand("set_person_group_members", () => null);
    mount();
    const target = (await screen.findByText("New group")).closest("li")!;
    dragOnto(screen.getByText("Anna").closest("li")!, target);
    await userEvent.type(await screen.findByRole("textbox", { name: "Group name" }), "Friends{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_person_group", { name: "Friends" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_group_members", { group: 9, people: [1], member: true }));
  });

  it("selects from a person's menu, clicks to add more, and ends the selection after an action", async () => {
    mockCommand("list_person_groups", () => [{ id: 7, name: "Family", personIds: [] }]);
    mockCommand("set_person_group_members", () => null);
    const { ContextMenuHost } = await import("@/components/context-menu");
    mount("/people", <ContextMenuHost />);
    fireEvent.contextMenu((await screen.findByText("Anna")).closest("li")!);
    await userEvent.click(await screen.findByRole("menuitem", { name: /^Select/ }));
    const bar = await screen.findByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveTextContent("1 person selected");
    await userEvent.click(screen.getByText("Add a name"));
    expect(bar).toHaveTextContent("2 people selected");
    await userEvent.click(screen.getByRole("button", { name: "Add to group" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Family" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_group_members", { group: 7, people: [1, 2], member: true }));
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Selection" })).not.toBeInTheDocument());
  });

  it("starts selecting with Ctrl-click and selects a range with Shift-click", async () => {
    mount("/people", null, () => {
      mockCommand("list_people", () => [...people, { id: 3, name: "Cara", faceCount: 5, photoCount: 4, coverFaceId: 30 }]);
      mockCommand("list_person_groups", () => []);
    });
    const link = (name: string) => screen.getByText(name).closest("a")!;
    await screen.findByText("Cara");
    fireEvent.click(link("Anna"), { ctrlKey: true });
    const bar = await screen.findByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveTextContent("1 person selected");
    fireEvent.click(link("Cara"), { shiftKey: true });
    expect(bar).toHaveTextContent("3 people selected");
    fireEvent.click(link("Add a name"), { ctrlKey: true });
    expect(bar).toHaveTextContent("2 people selected");
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: "Selection" })).not.toBeInTheDocument());
  });

  it("opens a group on its members, largest first", async () => {
    mockCommand("list_person_groups", () => [{ id: 7, name: "Family", personIds: [2, 1] }]);
    mount("/people?group=7");
    const list = await screen.findByRole("list", { name: "People in Family" });
    expect(list.querySelectorAll("li")[0]).toHaveTextContent("Anna");
    expect(screen.getByText("2 people")).toBeInTheDocument();
  });

  it("shows one person's photos and saves a new name", async () => {
    mockCommand("name_person", () => null);
    mount("/people/2");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ personId: 2 }) })));
    const field = await screen.findByRole("textbox", { name: "Name" });
    await userEvent.type(field, "Ben{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("name_person", { id: 2, name: "Ben" }));
  });

  it("shows a new name at once, while the library is still saving it (Round 5B)", async () => {
    mockCommand("name_person", () => new Promise(() => {}));
    mount("/people/1");
    const field = await screen.findByRole("textbox", { name: "Name" });
    await waitFor(() => expect(field).toHaveValue("Anna"));
    await userEvent.clear(field);
    await userEvent.type(field, "Anne{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("name_person", { id: 1, name: "Anne" }));
    expect(field).toHaveValue("Anne");
  });

  it("puts the old name back when saving a new one fails", async () => {
    mockCommand("name_person", () => Promise.reject({ code: "db", message: "locked" }));
    mount("/people/1");
    const field = await screen.findByRole("textbox", { name: "Name" });
    await waitFor(() => expect(field).toHaveValue("Anna"));
    await userEvent.clear(field);
    await userEvent.type(field, "Anne{Enter}");
    await waitFor(() => expect(field).toHaveValue("Anna"));
  });

  it("offers Undo for photos taken out of a person before the screen has refreshed", async () => {
    const places = [{ face: 7, person: 2, pinned: false }];
    mockCommand("move_person_photos", () => places);
    const rect = HTMLElement.prototype.getBoundingClientRect;
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 1000, height: 800, top: 0, left: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    onTestFinished(() => {
      HTMLElement.prototype.getBoundingClientRect = rect;
    });
    const { ContextMenuHost } = await import("@/components/context-menu");
    let refetching = false;
    mount("/people/2", <ContextMenuHost />, () => {
      mockCommand("get_layout", () => (refetching ? new Promise(() => {}) : { totalHeight: 200, itemCount: 1, rowHeights: [200], sections: [] }));
      mockCommand("get_layout_rows", () => [
        { index: 0, kind: "tiles", y: 0, height: 200, section: 0, title: null, tiles: [{ id: 42, x: 0, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 }] },
      ]);
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Item 42" }));
    refetching = true;
    await userEvent.click(await screen.findByText("Not this person"));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
  });

  it("takes photos out of a person, and offers Undo", async () => {
    const places = [{ face: 7, person: 2, pinned: false }];
    mockCommand("move_person_photos", () => places);
    mockCommand("restore_faces", () => null);
    const rect = HTMLElement.prototype.getBoundingClientRect;
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 1000, height: 800, top: 0, left: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    onTestFinished(() => {
      HTMLElement.prototype.getBoundingClientRect = rect;
    });
    const { ContextMenuHost } = await import("@/components/context-menu");
    mount("/people/2", <ContextMenuHost />, () => {
      mockCommand("get_layout", () => ({ totalHeight: 200, itemCount: 1, rowHeights: [200], sections: [] }));
      mockCommand("get_layout_rows", () => [
        { index: 0, kind: "tiles", y: 0, height: 200, section: 0, title: null, tiles: [{ id: 42, x: 0, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 }] },
      ]);
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Item 42" }));
    await userEvent.click(await screen.findByText("Not this person"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_person_photos", { person: 2, assetIds: [42], to: null }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_faces", { places }));
  });

  it("merges this person into a named one from the header", async () => {
    mockCommand("merge_people", () => ({ faces: [5], name: null, tookName: false, into: 1 }));
    mount("/people/2");
    await userEvent.click(await screen.findByRole("button", { name: "Merge with…" }));
    await userEvent.type(await screen.findByRole("textbox", { name: "Search people" }), "ann{Enter}");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("merge_people", { into: 1, from: 2 }));
  });

  describe("suggestions", () => {
    const suggest = () => mockCommand("person_suggestions", () => [
      { faceId: 51, assetId: 5, similarity: 0.41 },
      { faceId: 52, assetId: 6, similarity: 0.39 },
    ]);
    const undoOf = (a?: Record<string, unknown>) => ({ face: a?.face, person: a?.person, yes: a?.yes, before: { face: a?.face, person: null, pinned: false } });

    it("asks about doubtful faces, and a Yes takes the face out of the strip", async () => {
      mockCommand("answer_suggestion", undoOf);
      mount("/people/1", null, suggest);
      expect(await screen.findByText("Is this Anna?")).toBeInTheDocument();
      const [first] = await screen.findAllByRole("button", { name: "Yes, this is Anna" });
      await userEvent.click(first);
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("answer_suggestion", { face: 51, person: 1, yes: true }));
      await waitFor(() => expect(screen.getAllByRole("button", { name: "Yes, this is Anna" })).toHaveLength(1));
    });

    it("says No to all in one go, with Undo", async () => {
      mockCommand("answer_suggestion", undoOf);
      mockCommand("undo_suggestion", () => null);
      mount("/people/1", null, suggest);
      await userEvent.click(await screen.findByRole("button", { name: "No to all" }));
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("answer_suggestion", { face: 52, person: 1, yes: false }));
      expect(invokeMock).toHaveBeenCalledWith("answer_suggestion", { face: 51, person: 1, yes: false });
      await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
      toastStore.get().toasts.at(-1)?.action?.run();
      await waitFor(() => expect(invokeMock.mock.calls.filter(([c]) => c === "undo_suggestion")).toHaveLength(2));
    });

    it("puts a face back and says so when an answer fails", async () => {
      mockCommand("answer_suggestion", () => {
        throw { code: "db", message: "The library could not be changed." };
      });
      mount("/people/1", null, suggest);
      const [first] = await screen.findAllByRole("button", { name: "Yes, this is Anna" });
      await userEvent.click(first);
      await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("The library could not be changed."));
      expect(screen.getAllByRole("button", { name: "Yes, this is Anna" })).toHaveLength(2);
    });

    it("is not there with smart features off", async () => {
      mount("/people/1", null, () => {
        suggest();
        mockCommand("get_settings", () => ({ smart: { enabled: false } }));
      });
      await screen.findByRole("textbox", { name: "Name" });
      expect(screen.queryByText("Is this Anna?")).toBeNull();
      expect(invokeMock).not.toHaveBeenCalledWith("person_suggestions", expect.anything());
    });
  });

  it("has no Show hidden switch: nobody is hidden any more (Round 4B)", async () => {
    mount();
    await screen.findAllByRole("listitem");
    expect(screen.queryByRole("switch", { name: /hidden/i })).toBeNull();
    expect(screen.queryByText("Show hidden")).toBeNull();
  });

  it("says Not a person from the person's menu, goes back to People, and offers Undo", async () => {
    const undo = { id: 2, name: null, coverFaceId: 20, createdAt: 1, status: "listed", faces: [], rejections: [] };
    mockCommand("reject_person", () => undo);
    mockCommand("undo_reject_person", () => null);
    mount("/people/2");
    await userEvent.click(await screen.findByRole("button", { name: "More" }));
    expect(screen.queryByRole("menuitem", { name: /Hide/ })).toBeNull();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Not a person" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("reject_person", { id: 2 }));
    expect(await screen.findByRole("list", { name: "People" })).toBeInTheDocument();
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("undo_reject_person", { undo }));
  });

  it("removes a person from People, goes back, and offers Undo", async () => {
    mockCommand("remove_person", () => null);
    mockCommand("restore_removed_person", () => null);
    mount("/people/1");
    await userEvent.click(await screen.findByRole("button", { name: "More" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Remove from People" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("remove_person", { id: 1 }));
    expect(await screen.findByRole("list", { name: "People" })).toBeInTheDocument();
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restore_removed_person", { id: 1 }));
  });

  it("uses a photo's face as the person's main photo, with Undo", async () => {
    let cover = 20;
    mockCommand("set_person_cover", (a) => {
      const before = cover;
      cover = Number(a?.face);
      return before;
    });
    const rect = HTMLElement.prototype.getBoundingClientRect;
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 1000, height: 800, top: 0, left: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    onTestFinished(() => {
      HTMLElement.prototype.getBoundingClientRect = rect;
    });
    const { ContextMenuHost } = await import("@/components/context-menu");
    mount("/people/2", <ContextMenuHost />, () => {
      mockCommand("faces_for_asset", () => [{ id: 99, personId: 2, name: null, x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { id: 98, personId: 1, name: "Anna", x: 0.5, y: 0.1, w: 0.1, h: 0.1 }]);
      mockCommand("list_people", () => people.map((p) => (p.id === 2 ? { ...p, coverFaceId: cover } : p)));
      mockCommand("get_layout", () => ({ totalHeight: 200, itemCount: 1, rowHeights: [200], sections: [] }));
      mockCommand("get_layout_rows", () => [
        { index: 0, kind: "tiles", y: 0, height: 200, section: 0, title: null, tiles: [{ id: 42, x: 0, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 }] },
      ]);
    });
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Item 42" }));
    await userEvent.click(await screen.findByText("Use as main photo"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_cover", { person: 2, face: 99 }));
    await waitFor(() => expect(document.querySelector(".people-route__face--large")?.getAttribute("src")).toContain("face/99"));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo"));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_person_cover", { person: 2, face: 20 }));
  });

  it("says why the page is empty", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("list_people", () => []);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/people"]}>
          <Routes>
            <Route path="/people" element={<PeopleRoute />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("No people yet")).toBeInTheDocument();
  });
});
