import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { ProposalReviewItem } from "@/types/generated/ProposalReviewItem";
import type { ProposalValue } from "@/types/generated/ProposalValue";
import { ProposalReview } from "./gui";

vi.mock("maplibre-gl", async () => ({ Map: (await import("@/test/maplibre-mock")).FakeMap, setWorkerUrl: () => {}, addProtocol: () => {} }));

const row = (id: number, proposed: ProposalValue, current: ProposalValue | null, confidence = 0.9): ProposalReviewItem => ({
  proposal: { id, pluginId: "acme.finder", assetId: id * 10, proposed, confidence, reason: id === 1 ? "The file name says so" : null, status: "pending" },
  filename: `IMG_${id}.jpg`,
  current,
});

const titles = [
  row(1, { kind: "title", value: "Sunset" }, { kind: "title", value: "Old title" }, 0.95),
  row(2, { kind: "title", value: "Beach day" }, null, 0.8),
  row(3, { kind: "title", value: "Dusk" }, null, 0.91),
];

function open(kind: string, items: ProposalReviewItem[]) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => null);
  mockCommand("list_plugins", () => []);
  mockCommand("get_plugin_developer_mode", () => false);
  mockCommand("proposals_list", () => ({ items, nextCursor: null }));
  render(
    <Providers>
      <MemoryRouter initialEntries={[`/review/${kind}?plugin=acme.finder`]}>
        <Routes>
          <Route path="/review/:kind" element={<ProposalReview kind={kind as ProposalValue["kind"]} />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

const report = (accepted: number[], skipped: { id: number; reason: string }[] = []) => ({
  accepted,
  skipped,
  undo: { proposals: accepted, reversals: [] },
});

describe("ProposalReview", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    toastStore.set({ toasts: [] });
  });
  afterEach(cleanup);

  it("shows each photo with what it has now beside what the plugin proposes", async () => {
    open("title", titles);
    expect(await screen.findByText("IMG_1.jpg")).toBeInTheDocument();
    expect(screen.getByText("Sunset")).toBeInTheDocument();
    expect(screen.getByText("Old title")).toBeInTheDocument();
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText("The file name says so")).toBeInTheDocument();
    expect(screen.getAllByText("Nothing yet").length).toBe(2);
    expect(screen.getByRole("heading", { name: "Review titles" })).toBeInTheDocument();
  });

  it("Accept writes that one proposal and the toast takes it back", async () => {
    open("title", titles);
    mockCommand("proposals_accept", () => report([1]));
    mockCommand("proposals_undo_accept", () => null);
    await userEvent.click(await screen.findByRole("button", { name: "Accept IMG_1.jpg" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("proposals_accept", { items: [{ id: 1 }] }));
    const t = toastStore.get().toasts.at(-1);
    expect(t?.message).toBe("Accepted 1 title.");
    t?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("proposals_undo_accept", { undo: { proposals: [1], reversals: [] } }));
  });

  it("Reject leaves the proposal out and the toast brings it back", async () => {
    open("title", titles);
    mockCommand("proposals_reject", () => 1);
    mockCommand("proposals_restore", () => 1);
    await userEvent.click(await screen.findByRole("button", { name: "Reject IMG_2.jpg" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("proposals_reject", { ids: [2] }));
    toastStore.get().toasts.at(-1)?.action?.run();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("proposals_restore", { ids: [2] }));
  });

  it("Edit changes the value before it is accepted", async () => {
    open("title", titles);
    mockCommand("proposals_accept", () => report([2]));
    await userEvent.click(await screen.findByRole("button", { name: "Edit IMG_2.jpg" }));
    const box = screen.getByRole("textbox", { name: "Title for IMG_2.jpg" });
    await userEvent.clear(box);
    await userEvent.type(box, "Evening swim");
    await userEvent.click(screen.getByRole("button", { name: "Save and accept" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("proposals_accept", { items: [{ id: 2, value: { kind: "title", value: "Evening swim" } }] }),
    );
  });

  it("an edit that isn't a value of the kind can't be saved", async () => {
    open("location", [row(1, { kind: "location", value: { lat: 46.05, lon: 14.5 } }, null)]);
    await userEvent.click(await screen.findByRole("button", { name: "Edit IMG_1.jpg" }));
    const box = screen.getByRole("textbox", { name: "Place for IMG_1.jpg" });
    await userEvent.clear(box);
    await userEvent.type(box, "north");
    expect(screen.getByRole("button", { name: "Save and accept" })).toBeDisabled();
  });

  it("Accept all takes only the proposals at or above the chosen confidence", async () => {
    open("title", titles);
    mockCommand("proposals_accept", () => report([1, 3]));
    // The default bar is 90%: 95% and 91% pass, 80% doesn't.
    await userEvent.click(await screen.findByRole("button", { name: "Accept 2 above 90%" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("proposals_accept", { items: [{ id: 1 }, { id: 3 }] }));
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Accepted 2 titles.");
  });

  it("says why a proposal was skipped", async () => {
    open("location", [row(1, { kind: "location", value: { lat: 46.05, lon: 14.5 } }, { kind: "location", value: { lat: 1, lon: 2 } })]);
    mockCommand("proposals_accept", () => report([], [{ id: 1, reason: "That photo already has a location." }]));
    await userEvent.click(await screen.findByRole("button", { name: "Accept IMG_1.jpg" }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("That photo already has a location."));
  });

  it("a person's proposal shows the face and the name", async () => {
    open("person", [row(1, { kind: "person", value: { face: 7, name: "Ana" } }, null)]);
    expect(await screen.findByText("Ana")).toBeInTheDocument();
    expect(within(screen.getByRole("listitem")).getByRole("img", { name: "The face" }).getAttribute("src")).toContain("/face/7");
  });

  it("a place's proposal is drawn on the map", async () => {
    open("location", [row(1, { kind: "location", value: { lat: 46.05, lon: 14.5 } }, null)]);
    expect(await screen.findByText("46.0500, 14.5000")).toBeInTheDocument();
    expect(screen.getByTestId("proposal-map")).toBeInTheDocument();
  });

  it("an empty review is one sentence and a way back", async () => {
    open("date", []);
    expect(await screen.findByText("Nothing to review.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Accept \d/ })).toBeNull();
  });
});
