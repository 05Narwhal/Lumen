import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render as renderBare, screen, waitFor } from "@testing-library/react";
import { type ReactElement, type ReactNode, useCallback, useRef, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FrameKind } from "@/types/generated/FrameKind";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { drawFrame, SlideshowPreview } from "./gui";

/** The preview asks how each video clip plays (Round 6), which needs a query client. */
function Providers({ children }: { children: ReactNode }) {
  const [qc] = useState(() => new QueryClient({ defaultOptions: { queries: { retry: false } } }));
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}
const render = (ui: ReactElement) => renderBare(ui, { wrapper: Providers });

const doc = (kind: SlideshowDoc["clips"][number]["transition"]["kind"] = "crossfade", frame: FrameKind = "fill"): SlideshowDoc => ({
  version: 1,
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: [1, 2].map((asset_id) => ({
    asset_id,
    extra: [],
    frame,
    duration_ms: 4000,
    transition: { kind, duration_ms: 1000 },
    motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] },
  })),
});

/** Every blur radius the drawing asked for, so a test can prove the backdrop is blurred. */
let blurs: string[] = [];

function fakeContext() {
  blurs = [];
  const ctx = {
    save: vi.fn(),
    restore: vi.fn(),
    clearRect: vi.fn(),
    fillRect: vi.fn(),
    drawImage: vi.fn(),
    beginPath: vi.fn(),
    rect: vi.fn(),
    arc: vi.fn(),
    clip: vi.fn(),
    translate: vi.fn(),
    scale: vi.fn(),
    setTransform: vi.fn(),
    globalAlpha: 1,
    fillStyle: "",
    set filter(value: string) {
      if (value !== "none") blurs.push(value);
    },
    get filter() {
      return "none";
    },
  };
  return ctx as unknown as CanvasRenderingContext2D & { drawImage: ReturnType<typeof vi.fn>; fillRect: ReturnType<typeof vi.fn> };
}

const image = (width: number, height: number) => ({ width, height, complete: true }) as HTMLImageElement;

describe("drawFrame", () => {
  it("draws one picture covering the frame when nothing is in transition", () => {
    const ctx = fakeContext();
    drawFrame(ctx, doc(), 1000, { width: 800, height: 450 }, new Map([[1, image(1600, 900)]]));
    expect(ctx.drawImage).toHaveBeenCalledTimes(1);
    const [, dx, dy, dw, dh] = ctx.drawImage.mock.calls[0];
    expect([dx, dy, dw, dh]).toEqual([0, 0, 800, 450]);
  });

  it("covers the frame with a photo of another shape, overhanging it", () => {
    const ctx = fakeContext();
    drawFrame(ctx, doc(), 1000, { width: 800, height: 450 }, new Map([[1, image(1000, 1000)]]));
    const [, dx, dy, dw, dh] = ctx.drawImage.mock.calls[0];
    expect(dw).toBeCloseTo(800, 3);
    expect(dh).toBeCloseTo(800, 3);
    expect(dx).toBeCloseTo(0, 3);
    expect(dy).toBeCloseTo(-175, 3);
  });

  it("draws both pictures through a transition, the incoming one last", () => {
    const ctx = fakeContext();
    const images = new Map([
      [1, image(1600, 900)],
      [2, image(1600, 900)],
    ]);
    drawFrame(ctx, doc(), 3500, { width: 800, height: 450 }, images);
    expect(ctx.drawImage).toHaveBeenCalledTimes(2);
    expect(ctx.drawImage.mock.calls[1][0]).toBe(images.get(2));
  });

  it("puts a blurred backdrop behind a portrait and the whole photo in front", () => {
    const ctx = fakeContext();
    const portrait = image(900, 1600);
    drawFrame(ctx, doc("crossfade", "blur"), 1000, { width: 800, height: 450 }, new Map([[1, portrait]]));
    expect(ctx.drawImage).toHaveBeenCalledTimes(2);
    // The backdrop covers and overhangs; the sharp copy fits inside, full height.
    const [, , , backdropW, backdropH] = ctx.drawImage.mock.calls[0];
    expect(backdropW).toBeGreaterThanOrEqual(800);
    expect(backdropH).toBeGreaterThan(450);
    const [, sharpX, sharpY, sharpW, sharpH] = ctx.drawImage.mock.calls[1];
    expect(sharpH).toBeCloseTo(450, 3);
    expect(sharpW).toBeLessThan(800);
    expect(sharpY).toBeCloseTo(0, 3);
    expect(sharpX).toBeGreaterThan(0);
    expect(blurs).toContain("blur(20px)");
  });

  it("draws a grid clip as a backdrop and one photo per column", () => {
    const ctx = fakeContext();
    const images = new Map([
      [1, image(900, 1600)],
      [2, image(900, 1600)],
    ]);
    const grid = doc("crossfade", "grid");
    grid.clips[0].extra = [2];
    drawFrame(ctx, grid, 1000, { width: 800, height: 450 }, images);
    expect(ctx.drawImage).toHaveBeenCalledTimes(3);
    const left = ctx.drawImage.mock.calls[1];
    const right = ctx.drawImage.mock.calls[2];
    expect(left[0]).toBe(images.get(1));
    expect(right[0]).toBe(images.get(2));
    expect(left[1]).toBeLessThan(right[1]);
  });

  it("paints black under a fade through black", () => {
    const ctx = fakeContext();
    drawFrame(ctx, doc("fadeblack"), 3500, { width: 800, height: 450 }, new Map([[1, image(1600, 900)]]));
    expect(ctx.fillRect).toHaveBeenCalledWith(0, 0, 800, 450);
  });

  it("skips a picture that has not loaded yet instead of throwing", () => {
    const ctx = fakeContext();
    drawFrame(ctx, doc(), 1000, { width: 800, height: 450 }, new Map());
    expect(ctx.drawImage).not.toHaveBeenCalled();
  });

  /** A video element with a frame of this size, or none yet. */
  const video = (width: number, height: number, ready = true) => {
    const v = document.createElement("video");
    Object.defineProperty(v, "videoWidth", { value: width });
    Object.defineProperty(v, "videoHeight", { value: height });
    Object.defineProperty(v, "readyState", { value: ready ? 4 : 0 });
    return v;
  };
  const withVideo = (): SlideshowDoc => {
    const d = doc();
    d.clips[0] = { ...d.clips[0], video: { in_ms: 2000, out_ms: 6000 } };
    return d;
  };

  it("paints a video clip from its video", () => {
    const ctx = fakeContext();
    const v = video(1920, 1080);
    const poster = image(1600, 900);
    drawFrame(ctx, withVideo(), 1000, { width: 800, height: 450 }, new Map([[1, poster]]), new Map([[1, v]]));
    expect(ctx.drawImage).toHaveBeenCalledTimes(1);
    expect(ctx.drawImage.mock.calls[0][0]).toBe(v);
  });

  it("shows a video's poster until the video has a frame", () => {
    const ctx = fakeContext();
    const poster = image(1600, 900);
    drawFrame(ctx, withVideo(), 1000, { width: 800, height: 450 }, new Map([[1, poster]]), new Map([[1, video(1920, 1080, false)]]));
    expect(ctx.drawImage.mock.calls[0][0]).toBe(poster);
  });
});

describe("SlideshowPreview", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(fakeContext() as unknown as null);
  });

  it("renders a canvas of the document's shape", () => {
    render(<SlideshowPreview doc={doc()} slideshowId={1} timeMs={0} playing={false} onTime={() => {}} />);
    const canvas = screen.getByLabelText("Slideshow preview");
    expect(canvas.tagName).toBe("CANVAS");
    expect(canvas.parentElement).toHaveStyle({ aspectRatio: "1920 / 1080" });
  });

  const track = (path: string) => ({ path, fade_in_ms: 500, fade_out_ms: 500, gain_db: 0, start_ms: 0 });

  it("plays a track from outside the library through the show, named in the URL", () => {
    const withAudio = { ...doc(), audio: track("C:/music/song.mp3") };
    const { container } = render(<SlideshowPreview doc={withAudio} slideshowId={7} timeMs={0} playing={false} onTime={() => {}} />);
    const src = container.querySelector("audio")?.getAttribute("src") ?? "";
    expect(src).toContain("slideshow-audio/7");
    expect(src).toContain(`t=${encodeURIComponent("C:/music/song.mp3")}`);
  });

  /**
   * The first-play bug: the show's audio URL was served from the *saved* show, and the
   * editor saves 600 ms after a change — so a track just picked was asked for before the
   * show had it, failed, and was never asked for again.
   */
  it("plays a library track by its own name, so it works before the show is saved", () => {
    const { container } = render(
      <SlideshowPreview doc={{ ...doc(), audio: track("music/1886711.mp3") }} slideshowId={7} timeMs={0} playing={false} onTime={() => {}} />,
    );
    expect(container.querySelector("audio")?.getAttribute("src")).toMatch(/library-music\/1886711\.mp3$/);
  });

  it("recognises a library track saved with a Windows separator", () => {
    const { container } = render(
      <SlideshowPreview doc={{ ...doc(), audio: track("music\\1886711.mp3") }} slideshowId={7} timeMs={0} playing={false} onTime={() => {}} />,
    );
    expect(container.querySelector("audio")?.getAttribute("src")).toMatch(/library-music\/1886711\.mp3$/);
  });

  it("asks again when a file from this computer fails because the show is not saved yet", () => {
    vi.useFakeTimers();
    try {
      const { container } = render(
        <SlideshowPreview doc={{ ...doc(), audio: track("D:/Music/song.mp3") }} slideshowId={7} timeMs={0} playing={false} onTime={() => {}} />,
      );
      const first = container.querySelector("audio")!;
      act(() => {
        first.dispatchEvent(new Event("error"));
      });
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      const again = container.querySelector("audio")!;
      expect(again).not.toBe(first);
      expect(again.getAttribute("src")).toContain("r=1");
    } finally {
      vi.useRealTimers();
    }
  });

  it("swaps the element when the track changes", () => {
    const props = { slideshowId: 7, timeMs: 0, playing: false, onTime: () => {} };
    const { container, rerender } = render(<SlideshowPreview doc={{ ...doc(), audio: track("music/a.mp3") }} {...props} />);
    const first = container.querySelector("audio");
    rerender(<SlideshowPreview doc={{ ...doc(), audio: track("music/b.mp3") }} {...props} />);
    const second = container.querySelector("audio");
    expect(second).not.toBe(first);
    expect(second?.getAttribute("src")).toMatch(/library-music\/b\.mp3$/);
  });

  it("starts a new track where the show is, not at its beginning", () => {
    const set = vi.fn();
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", { configurable: true, get: () => 0, set });
    const props = { slideshowId: 7, timeMs: 5000, playing: false, onTime: () => {} };
    const { rerender } = render(<SlideshowPreview doc={{ ...doc(), audio: track("music/a.mp3") }} {...props} />);
    set.mockClear();
    rerender(<SlideshowPreview doc={{ ...doc(), audio: track("music/b.mp3") }} {...props} />);
    expect(set).toHaveBeenCalledWith(5);
  });

  it("starts a track added while the show is playing", () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(() => Promise.resolve());
    const props = { slideshowId: 7, timeMs: 0, playing: true, onTime: () => {} };
    const { rerender } = render(<SlideshowPreview doc={doc()} {...props} />);
    expect(play).not.toHaveBeenCalled();
    rerender(<SlideshowPreview doc={{ ...doc(), audio: track("music/a.mp3") }} {...props} />);
    expect(play).toHaveBeenCalledTimes(1);
    play.mockRestore();
  });

  it("has no audio element without a track", () => {
    const { container } = render(<SlideshowPreview doc={doc()} slideshowId={1} timeMs={0} playing={false} onTime={() => {}} />);
    expect(container.querySelector("audio")).toBeNull();
  });
});

/**
 * The track and the picture run off one clock, and the track is seeked only when the time
 * comes from somewhere other than that clock.
 *
 * This is where "the music stutters" came from. The preview compared `audio.currentTime`
 * against the show's time and seeked whenever they differed by more than a moment — but a
 * beat-synced show trims the track's head by `start_ms`, so the two are *always* that far
 * apart, and every single animation frame seeked the element. Each seek restarts the
 * decoder, so the track never played more than one frame's worth before being thrown back.
 */
describe("SlideshowPreview's audio clock", () => {
  let seeks: number[] = [];
  let plays = 0;
  let pauses = 0;

  const track = (start_ms: number) => ({ path: "music/song.mp3", fade_in_ms: 0, fade_out_ms: 0, gain_db: 0, start_ms });
  const withAudio = (start_ms: number): SlideshowDoc => ({ ...doc(), audio: track(start_ms) });

  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(fakeContext() as unknown as null);
    seeks = [];
    plays = 0;
    pauses = 0;
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get() {
        return seeks.length ? seeks[seeks.length - 1] : 0;
      },
      set(v: number) {
        seeks.push(v);
      },
    });
    Object.defineProperty(HTMLMediaElement.prototype, "play", {
      configurable: true,
      value: () => {
        plays += 1;
        return Promise.resolve();
      },
    });
    Object.defineProperty(HTMLMediaElement.prototype, "pause", { configurable: true, value: () => void (pauses += 1) });
  });

  /** The editor: it owns the time, and playback reports back into it. */
  function Harness({ docu, playing }: { docu: SlideshowDoc; playing: boolean }) {
    const [t, setT] = useState(0);
    return <SlideshowPreview doc={docu} slideshowId={3} timeMs={t} playing={playing} onTime={setT} />;
  }

  /**
   * The editor as it behaves at a transition: drawing two layers with blur is slow, so the
   * render carrying a reported time lands frames after the clock moved on. That late time
   * is still the preview's own and must not be taken for a seek. It was — the clock and
   * the track jumped back to it, and the music repeated a moment, once or several times.
   */
  function LaggingHarness({ docu }: { docu: SlideshowDoc }) {
    const [t, setT] = useState(0);
    const reported = useRef<number[]>([]);
    const onTime = useCallback((ms: number) => {
      reported.current.push(ms);
      const late = reported.current[reported.current.length - 3];
      if (late !== undefined) setT(late);
    }, []);
    return <SlideshowPreview doc={docu} slideshowId={3} timeMs={t} playing onTime={onTime} />;
  }

  it("never seeks the track back when a render lands late", async () => {
    render(<LaggingHarness docu={withAudio(1500)} />);
    await waitFor(() => expect(plays).toBeGreaterThan(0));
    await new Promise((r) => setTimeout(r, 200));
    expect(seeks).toEqual([1.5]);
  });

  it("follows the track while it plays, so the picture keeps time with the music", async () => {
    let audioNow = 0;
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get: () => audioNow,
      set(v: number) {
        seeks.push(v);
        audioNow = v;
      },
    });
    Object.defineProperty(HTMLMediaElement.prototype, "paused", { configurable: true, get: () => plays <= pauses });
    Object.defineProperty(HTMLMediaElement.prototype, "readyState", { configurable: true, get: () => 4 });
    const times: number[] = [];
    render(<SlideshowPreview doc={withAudio(1500)} slideshowId={3} timeMs={0} playing onTime={(ms) => times.push(ms)} />);
    await waitFor(() => expect(plays).toBeGreaterThan(0));
    // The track has played 1.2 s past the trim: the show is 1.2 s in, whatever the frames did.
    audioNow = 2.7;
    await waitFor(() => expect(times.at(-1)).toBeCloseTo(1200, -1));
    expect(seeks).toEqual([1.5]);
  });

  it("starts the track at the trim the beat sync chose, not at zero", () => {
    render(<SlideshowPreview doc={withAudio(1500)} slideshowId={3} timeMs={0} playing={false} onTime={() => {}} />);
    expect(seeks).toEqual([1.5]);
  });

  it("seeks once for a whole run, however many frames it draws", async () => {
    render(<Harness docu={withAudio(1500)} playing />);
    await waitFor(() => expect(plays).toBeGreaterThan(0));
    // Let the clock run: several animation frames, each of which used to seek.
    await new Promise((r) => setTimeout(r, 120));
    expect(seeks).toEqual([1.5]);
  });

  it("plays and pauses once per change, not once per frame", async () => {
    const { rerender } = render(<Harness docu={withAudio(0)} playing />);
    await waitFor(() => expect(plays).toBe(1));
    await new Promise((r) => setTimeout(r, 120));
    expect(plays).toBe(1);
    rerender(<Harness docu={withAudio(0)} playing={false} />);
    expect(pauses).toBeGreaterThan(0);
  });

  it("follows the track to a time the transport moved to", () => {
    const { rerender } = render(<SlideshowPreview doc={withAudio(1500)} slideshowId={3} timeMs={0} playing={false} onTime={() => {}} />);
    rerender(<SlideshowPreview doc={withAudio(1500)} slideshowId={3} timeMs={2000} playing={false} onTime={() => {}} />);
    expect(seeks).toEqual([1.5, 3.5]);
  });
});
