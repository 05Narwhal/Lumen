import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";
import { TextSection } from "./text-section";

/** A level line whose top is at `y`, 0.05 tall: lines 0.06 apart run on, a bigger gap ends a block. */
const at = (ord: number, text: string, y: number): ShownTextLine => ({ ord, text, quad: [0.1, y, 0.9, y, 0.9, y + 0.05, 0.1, y + 0.05] });

const lines = [at(0, "Gostilna", 0.1), at(1, "Kosilo 12 €", 0.5)];
// Five sentences, one per line.
const many = ["Ena.", "Dva.", "Tri.", "Štiri.", "Pet."].map((t, i) => at(i, t, 0.1 + i * 0.06));

const props = { reading: false, shown: false, onShowOnPhoto: () => {}, onMenu: () => {} };

describe("TextSection", () => {
  it("heads the row with the line count, singular too", () => {
    const { rerender } = render(<TextSection lines={lines} {...props} />);
    expect(screen.getByText("Text")).toBeInTheDocument();
    expect(screen.getByText("2 lines")).toBeInTheDocument();
    rerender(<TextSection lines={[lines[0]]} {...props} />);
    expect(screen.getByText("1 line")).toBeInTheDocument();
  });

  it("shows the first two sentences in the inset", () => {
    render(<TextSection lines={many} {...props} />);
    const inset = screen.getByTestId("text-inset");
    expect(within(inset).getAllByRole("paragraph").map((p) => p.textContent)).toEqual(["Ena.", "Dva."]);
    expect(screen.queryByText("Tri.")).toBeNull();
  });

  it("joins lines that run on into one sentence", () => {
    const run = [at(0, "Gostilna pri", 0.1), at(1, "Mostu je odprta.", 0.16)];
    render(<TextSection lines={run} {...props} />);
    expect(within(screen.getByTestId("text-inset")).getByText("Gostilna pri Mostu je odprta.")).toBeInTheDocument();
  });

  it("shows all the lines in place, then less", async () => {
    render(<TextSection lines={many} {...props} />);
    await userEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(screen.getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Ena.", "Dva.", "Tri.", "Štiri.", "Pet."]);
    expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(screen.queryAllByRole("listitem")).toHaveLength(0);
    expect(screen.queryByText("Tri.")).toBeNull();
  });

  it("offers no Show all when the inset already holds everything", () => {
    render(<TextSection lines={lines} {...props} />);
    expect(screen.queryByRole("button", { name: /Show (all|less)/ })).toBeNull();
  });

  it("copies every line, one per line, in reading order", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<TextSection lines={lines} {...props} />);
    await userEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("Gostilna\nKosilo 12 €"));
  });

  it("puts the text on the photo, and says when it is on", async () => {
    const onShowOnPhoto = vi.fn();
    const { rerender } = render(<TextSection lines={lines} {...props} onShowOnPhoto={onShowOnPhoto} />);
    const btn = screen.getByRole("button", { name: /On photo/ });
    expect(btn).toHaveAttribute("aria-pressed", "false");
    expect(btn).toHaveTextContent("T");
    await userEvent.click(btn);
    expect(onShowOnPhoto).toHaveBeenCalledTimes(1);
    rerender(<TextSection lines={lines} {...props} shown onShowOnPhoto={onShowOnPhoto} />);
    expect(screen.getByRole("button", { name: /On photo/ })).toHaveAttribute("aria-pressed", "true");
  });

  it("opens the text menu on the inset's text, with a selection if there is one", () => {
    const onMenu = vi.fn();
    render(<TextSection lines={lines} {...props} onMenu={onMenu} />);
    fireEvent.contextMenu(screen.getByText("Kosilo 12 €"));
    expect(onMenu).toHaveBeenCalledWith("Kosilo 12 €", expect.anything());
  });

  it("opens the text menu on a line when all are shown", async () => {
    const onMenu = vi.fn();
    render(<TextSection lines={many} {...props} onMenu={onMenu} />);
    await userEvent.click(screen.getByRole("button", { name: "Show all" }));
    fireEvent.contextMenu(screen.getByText("Tri."));
    expect(onMenu).toHaveBeenCalledWith("Tri.", expect.anything());
  });

  it("says it is reading, without a spinner", () => {
    const { container } = render(<TextSection lines={[]} {...props} reading />);
    expect(screen.getByText("Reading…")).toBeInTheDocument();
    expect(container.querySelector(".spinner")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("is not there for a photo without text", () => {
    const { container } = render(<TextSection lines={[]} {...props} />);
    expect(container).toBeEmptyDOMElement();
  });
});
