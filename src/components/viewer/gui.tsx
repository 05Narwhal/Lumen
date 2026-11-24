import {
  IconAperture,
  IconAdjustmentsHorizontal,
  IconFileExport,
  IconArrowsMaximize,
  IconChevronLeft,
  IconChevronRight,
  IconFolderOpen,
  IconInfoCircle,
  IconMaximize,
  IconMinimize,
  IconStar,
  IconStarFilled,
  IconTextScan2,
  IconTrash,
  IconX,
  IconZoomIn,
} from "@tabler/icons-react";
import { Tooltip } from "@mantine/core";
import { Spinner } from "@/components/spinner";
import { AnimatePresence, motion } from "motion/react";
import { playStarBurst } from "@/components/star-burst";
import { useCallback, useEffect, useRef, useState } from "react";
import { useMenuContributions } from "@/lib/plugin-slots";
import { useNavigate, useSearchParams } from "react-router";
import { openDevelop } from "@/lib/develop-source";
import { toast } from "@/components/toast";
import { useAsset, useNeighbors, useScopeNeighbors, useSectionNeighbors } from "@/hooks/use-asset";
import { clearViewerScope, viewerScopeStore } from "@/lib/viewer-scope";
import { noteRadarViewer } from "@/lib/radar-view";
import { useRatings } from "@/hooks/use-ratings";
import { StarRating } from "@/components/star-rating";
import { invoke, isAppError } from "@/lib/ipc";
import { setWindowFullscreen } from "@/lib/fullscreen";
import { clearViewerSequence, dropFromViewerSequence, useSequenceNeighbors, viewerSequenceIds } from "@/lib/viewer-sequence";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import { formatCaptured } from "@/utils/date";
import { useEditVersion } from "@/lib/edit-versions";
import { mediaUrl } from "@/utils/media";
import { openExportSheet } from "@/components/export-sheet";
import { useSavedRecipe } from "@/hooks/use-edit-session";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useContextMenu } from "@/components/context-menu";
import { useOpenWith } from "@/lib/external-apps";
import { faceMenu, photoTextMenu, viewerMenu } from "@/lib/menus";
import { useAssetText } from "@/hooks/use-asset-text";
import { allText, copyText, shownAspectOf, textOverlayRequest } from "@/lib/photo-text";
import { TextOverlay } from "./text-overlay";
import { openAlbumPicker } from "@/components/album-picker/store";
import { isMac } from "@/components/window-controls";
import { queryKeys } from "@/lib/query-keys";
import { rotatePhotos } from "@/lib/rotate";
import { inHiddenFolder, moveToHidden } from "@/lib/vault";
import type { FacePlace } from "@/types/generated/FacePlace";
import { PersonPicker } from "@/components/person-picker";
import { useFaceTags, usePeople, usePeopleActions, type TagTarget } from "@/hooks/use-people";
import type { Rect } from "@/lib/face-geometry";
import type { AssetFace } from "@/types/generated/AssetFace";
import { faceTaggingRequest, petBoxesRequest } from "@/lib/face-tagging";
import { FaceBoxes } from "./face-boxes";
import { EditMode } from "./edit-mode";
import { InfoPanel, type FaceTools, type PetTools } from "./info-panel";
import { usePetBoxMenu } from "@/hooks/use-pet-box-menu";
import type { AssetPet } from "@/types/generated/AssetPet";
import { LiveButton, LiveMotionLayer, useLiveMotion } from "./live-photo";
import { VideoPlayer } from "./video-player";
import { ZoomPan, type ZoomPanHandle } from "./zoom-pan";
import { isOversized, shownPercent } from "@/lib/oversized";
import type { Size } from "./zoom-math";
import { useIdle } from "@/hooks/use-idle";
import { matches } from "@/lib/keybinds";
import { isTyping } from "@/lib/typing";
import { WindowChrome } from "@/components/window-chrome";
import { useWindowDrag } from "@/hooks/use-window-drag";
import "./styles.scss";

/**
 * Extensions the webview decodes itself, so serving the original keeps its gain map or
 * PQ transfer. Mirrors `photos_media::hdr::browser_can_render`; HEIC is absent because
 * Chromium cannot read it at all.
 */
const BROWSER_FORMATS = new Set(["jpg", "jpeg", "jpe", "png", "avif", "webp", "gif"]);

/** True when we must hand the webview the file's own bytes to keep its dynamic range. */
export function servesOriginalForHdr(hdr: number, extension: string): boolean {
  return hdr > 0 && BROWSER_FORMATS.has(extension.toLowerCase());
}

/**
 * thumb 1024 → preview 2560 → full resolution when zoomed past fit.
 *
 * An HDR photo skips the 2560 preview: that preview is a re-encoded SDR JPEG and would
 * throw the extra range away. `full/{id}` serves the original for every format the
 * webview can decode, so the picture arrives with its gain map intact.
 */
function useProgressiveSrc(id: number | null, renderable: boolean, zoomed: boolean, hdrOriginal = false, oversized = false) {
  const [stage, setStage] = useState<"thumb" | "preview" | "original">("thumb");
  const [loading, setLoading] = useState(false);
  // The size of the full image once it has loaded (an oversized photo may come smaller than it is).
  const [fullSize, setFullSize] = useState<Size | null>(null);
  // A saved edit or a revert changes every URL of the photo: load it again from the top.
  const version = useEditVersion(id ?? -1);
  useEffect(() => {
    setStage("thumb");
    setLoading(false);
    setFullSize(null);
  }, [id, version]);
  useEffect(() => {
    if (id === null || !renderable) return;
    // An oversized photo asks for its full resolution as soon as the preview is up (Round 22 §2).
    const want = zoomed || hdrOriginal || (oversized && stage === "preview") ? "original" : "preview";
    if (stage === "original" || stage === want) return;
    const url = want === "original" ? mediaUrl("full", id) : mediaUrl("preview", id, { s: 2560 });
    const img = new Image();
    let alive = true;
    setLoading(true);
    img.onload = () => {
      if (!alive) return;
      if (want === "original") setFullSize({ width: img.naturalWidth, height: img.naturalHeight });
      setStage(want);
      setLoading(false);
    };
    img.onerror = () => alive && setLoading(false);
    img.src = url;
    return () => {
      alive = false;
      // Closing the viewer, or moving to another photo, drops a big download.
      if (oversized) img.src = "";
    };
  }, [id, renderable, zoomed, stage, hdrOriginal, oversized, version]);
  const src =
    id === null
      ? ""
      : stage === "original"
        ? mediaUrl("full", id)
        : stage === "preview"
          ? mediaUrl("preview", id, { s: 2560 })
          : mediaUrl("thumb", id, { s: 1024 });
  return { src, loading, stage, fullSize };
}

function ToolButton({ label, shortcut, pressed, danger, onClick, children }: { label: string; shortcut?: string; pressed?: boolean; danger?: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tooltip label={shortcut ? `${label} (${shortcut})` : label} openDelay={600} withArrow>
      <button type="button" className="viewer__icon" aria-label={label} aria-pressed={pressed} data-danger={danger} onClick={onClick}>
        {children}
      </button>
    </Tooltip>
  );
}

/**
 * Paging between pictures. A small slide plus a fade: enough to show which way the
 * timeline moved without throwing the picture across the screen.
 */
const PAGE = {
  enter: (d: number) => ({ opacity: 0, x: d * 48, scale: 0.98 }),
  centre: { opacity: 1, x: 0, scale: 1 },
  leave: (d: number) => ({ opacity: 0, x: d * -48, scale: 0.98 }),
};

/** Playing a memory: pictures dissolve into each other instead of travelling. */
const FADE = { enter: { opacity: 0 }, centre: { opacity: 1 }, leave: { opacity: 0 } };
const PLAY_EVERY_MS = 4000;

export function Viewer() {
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const raw = params.get("asset");
  const id = raw && /^\d+$/.test(raw) ? Number(raw) : null;
  const open = id !== null;
  const asset = useAsset(id);
  const live = useLiveMotion(asset.data?.liveVideoId ?? null);
  // A screen that opened the viewer on a set of its own (the Map) keeps paging inside it;
  // otherwise it pages inside the grid that opened it (an album in its order, a person, a
  // search), and walks the whole timeline only when there is no grid or the photo left it.
  const sequence = useSequenceNeighbors(id);
  const gridScope = viewerScopeStore.use();
  // A collection's photos by album: a photo can be in two sections, so the URL says which copy is open.
  const sectionRaw = params.get("section");
  const section = sectionRaw && /^\d+$/.test(sectionRaw) ? Number(sectionRaw) : null;
  const bySection = !sequence && gridScope?.scope.collectionId != null;
  const slots = useSectionNeighbors(bySection ? gridScope : null, id, section);
  const inScope = useScopeNeighbors(sequence || bySection ? null : gridScope, id);
  const scopedQuery = bySection ? slots : inScope;
  const scoped = !sequence && gridScope !== null && (scopedQuery.isPending || scopedQuery.data != null);
  const timeline = useNeighbors(sequence || scoped ? null : id);
  // The face radar checks this grid, nearest to the open photo first (Round 10).
  useEffect(() => {
    noteRadarViewer(id === null ? null : { scope: scoped ? gridScope : null, at: id });
  }, [id, scoped, gridScope]);
  useEffect(() => () => noteRadarViewer(null), []);
  const inScopeIds = bySection ? (slots.data ? { prev: slots.data.prev?.id ?? null, next: slots.data.next?.id ?? null } : undefined) : (inScope.data ?? undefined);
  const neighbors = { data: sequence ? { prev: sequence.prev, next: sequence.next } : scoped ? inScopeIds : timeline.data };
  const [fullscreen, setFullscreen] = useState(false);
  const toggleFullscreen = useCallback(() => {
    const next = !fullscreen;
    setFullscreen(next);
    void setWindowFullscreen(next);
  }, [fullscreen]);
  const [info, setInfo] = useState(false);
  // Chrome auto-hide after 2 s idle: a pointer move, a press or a key brings it back.
  const [idle, setIdle] = useIdle(open);
  const [scale, setScale] = useState(1);
  const zoom = useRef<ZoomPanHandle>(null);
  const isVideo = asset.data?.mediaType === 1;
  const renderable = (asset.data?.renderable ?? true) && !isVideo;
  // An edited photo is shown as rendered, never as its original file, HDR or not.
  const edited = useEditVersion(id ?? -1) !== undefined;
  const hdrOriginal = !edited && servesOriginalForHdr(asset.data?.hdr ?? 0, asset.data?.extension ?? "");
  // Stills can be edited, Live Photos included (their motion stays as it was).
  // Photos open the quick editor, videos the trimmer.
  const editable = !!asset.data && (asset.data.mediaType === 1 || asset.data.renderable);
  // A video's trim, or a still's geometry (face boxes are drawn through it).
  const savedRecipe = useSavedRecipe(id ?? 0, asset.data?.mediaType === 1 || edited);
  const editing = editable && params.get("edit") === "1";
  const oversized = isOversized(asset.data?.width, asset.data?.height);
  const { src, loading, stage, fullSize } = useProgressiveSrc(id, renderable, scale > 1.01, hdrOriginal, oversized);
  const shown = oversized && stage === "original" && fullSize && asset.data?.width && asset.data?.height ? shownPercent(fullSize, { width: asset.data.width, height: asset.data.height }) : null;

  // Faces (Round 4I): boxes over the photo, tagging a found face, drawing a missed one.
  const taggable = !!asset.data && asset.data.mediaType !== 1 && asset.data.renderable;
  const faces = useQuery({ queryKey: ["people", "faces", id], queryFn: () => invoke<AssetFace[]>("faces_for_asset", { id }), enabled: id !== null && taggable });
  const [showFaces, setShowFaces] = useState(false);
  const [drawing, setDrawing] = useState(false);
  const [hoverFace, setHoverFace] = useState<number | null>(null);
  const [tagging, setTagging] = useState<{ face: AssetFace } | { rect: Rect } | null>(null);
  const { people } = usePeople();
  const faceTags = useFaceTags();
  useEffect(() => {
    setDrawing(false);
    setHoverFace(null);
    setTagging(null);
  }, [id]);
  // *Tag people…* from the grid's menu or the palette.
  const tagRequest = faceTaggingRequest.use();
  useEffect(() => {
    if (tagRequest === null || tagRequest !== id || !taggable) return;
    faceTaggingRequest.set(null);
    setShowFaces(true);
    setInfo(true);
  }, [tagRequest, id, taggable]);
  const faceTools: FaceTools = {
    show: showFaces,
    onToggleShow: () => setShowFaces((v) => !v),
    drawing,
    onAddFace: () => {
      zoom.current?.zoomTo("fit");
      setDrawing((v) => !v);
    },
    onTag: (face) => setTagging({ face }),
    onUntag: (face) => void faceTags.untag(face),
    onHover: setHoverFace,
  };
  // A face box's own menu (Round 4G): the same tagging, and *Not a face* for a detection.
  const { open: openFaceMenu } = useContextMenu();
  const qc = useQueryClient();
  const peopleActions = usePeopleActions();
  const faceMenuFor = (face: AssetFace) =>
    faceMenu(face, {
      tag: () => setTagging({ face }),
      newPerson: () => setTagging({ face }),
      untag: () => void faceTags.untag(face),
      useAsMain: face.personId != null ? () => void peopleActions.setCover(face.personId!, face.id) : undefined,
      notAFace: face.manual
        ? undefined
        : async () => {
            try {
              const place = await invoke<FacePlace>("reject_face", { face: face.id });
              void qc.invalidateQueries({ queryKey: ["people"] });
              toast({
                message: "Not a face",
                undo: () => void invoke("undo_reject_face", { place }).then(() => qc.invalidateQueries({ queryKey: ["people"] })),
              });
            } catch (e) {
              toast({ message: isAppError(e) ? e.message : "Could not change that face." });
            }
          },
    });
  // Pets (Round 8B): their boxes on the same layer as the faces', each with its menu.
  const pets = useQuery({ queryKey: ["pets", "in", id], queryFn: () => invoke<AssetPet[]>("asset_pets", { asset: id }), enabled: id !== null && taggable });
  const [showPets, setShowPets] = useState(false);
  const [hoverPet, setHoverPet] = useState<number | null>(null);
  useEffect(() => setHoverPet(null), [id]);
  const petBoxMenu = usePetBoxMenu();
  const petRequest = petBoxesRequest.use();
  useEffect(() => {
    if (petRequest === null || petRequest !== id || !taggable) return;
    petBoxesRequest.set(null);
    setShowPets((v) => !v);
  }, [petRequest, id, taggable]);
  const petTools: PetTools ={ show: showPets, onToggleShow: () => setShowPets((v) => !v), onHover: setHoverPet };
  // Text in photos (Round 13-3): the lines on the photo as shown, the overlay and its menu.
  // The overlay stays on while paging, but only shows on a photo that has lines.
  const text = useAssetText(id, taggable);
  const hasText = text.lines.length > 0;
  const [showText, setShowText] = useState(false);
  const textShown = showText && hasText;
  const textRequest = textOverlayRequest.use();
  useEffect(() => {
    if (textRequest === null || textRequest !== id) return;
    textOverlayRequest.set(null);
    setShowText((v) => !v);
  }, [textRequest, id]);
  const copyOut = (t: string) => void copyText(t).then((ok) => toast({ message: ok ? "Copied" : "Could not copy the text." }));
  const textMenuFor = (t: string | null) =>
    photoTextMenu(
      { text: t },
      {
        copy: () => {
          if (t) copyOut(t);
        },
        copyAll: () => copyOut(allText(text.lines)),
        search: (q) => navigate(`/search?q=${encodeURIComponent(q)}`),
      },
    );
  const tagAs = (target: TagTarget) => {
    const t = tagging;
    setTagging(null);
    if (!t || id === null) return;
    if ("face" in t) void faceTags.tag(t.face, target);
    else void faceTags.addBox(id, t.rect, target);
  };
  const shownAspect = asset.data?.width && asset.data?.height ? (asset.data.orientation >= 5 ? asset.data.height / asset.data.width : asset.data.width / asset.data.height) : 1;

  // Warm the neighbours' viewer sizes so arrow-key paging is instant.
  useEffect(() => {
    const ids = [neighbors.data?.prev, neighbors.data?.next].filter((n): n is number => n != null);
    if (!ids.length) return;
    invoke("prefetch_previews", { ids }).catch(() => {});
    // A video beside this one is ready to play when paged to (Round 8A).
    invoke("prefetch_playback", { ids }).catch(() => {});
    // Their text, read ahead while reading text is on (Round 13-3).
    if (text.on) invoke("prefetch_text", { ids }).catch(() => {});
  }, [neighbors.data?.prev, neighbors.data?.next, text.on]);

  const close = useCallback(() => {
    const closing = id;
    clearViewerSequence();
    clearViewerScope();
    if (fullscreen) {
      setFullscreen(false);
      void setWindowFullscreen(false);
    }
    setParams(
      (p) => {
        p.delete("asset");
        p.delete("section");
        p.delete("edit");
        p.delete("play");
        return p;
      },
      { replace: true },
    );
    if (closing !== null) requestAnimationFrame(() => document.getElementById(`tile-${closing}`)?.querySelector("button")?.focus());
  }, [id, setParams, fullscreen]);

  // Which way the last move went: the outgoing picture leaves the side the new one
  // arrives from, so paging reads as travel along the timeline rather than a swap.
  const [dir, setDir] = useState(1);
  const go = useCallback(
    (to: number | null | undefined, direction = 1) => {
      if (to == null) return;
      setDir(direction);
      setParams(
        (p) => {
          p.set("asset", String(to));
          // Onward through a collection's sections: the neighbour's own copy.
          const near = direction < 0 ? [slots.data?.prev, slots.data?.next] : [slots.data?.next, slots.data?.prev];
          const slot = near.find((s) => s?.id === to);
          if (slot) p.set("section", String(slot.section));
          else p.delete("section");
          return p;
        },
        { replace: true },
      );
    },
    [setParams, slots.data],
  );

  const { setRating } = useRatings();
  const [rating, setRatingState] = useState(0);
  useEffect(() => setRatingState(asset.data?.rating ?? 0), [asset.data?.rating, id]);
  const rate = useCallback(
    (value: number) => {
      if (id === null) return;
      const previous = rating;
      setRatingState(value);
      void setRating([id], value, previous);
    },
    [id, rating, setRating],
  );

  const [favorite, setFavorite] = useState(false);
  useEffect(() => setFavorite(asset.data?.isFavorite ?? false), [asset.data?.isFavorite, id]);
  const stageRef = useRef<HTMLDivElement>(null);
  const toggleFavorite = useCallback(() => {
    if (id === null) return;
    const next = !favorite;
    setFavorite(next);
    // Over the picture itself, which is what the gesture is about.
    if (next) playStarBurst(stageRef.current);
    invoke("set_favorite", { ids: [id], value: next }).catch(() => setFavorite(!next));
  }, [id, favorite]);
  const trash = useCallback(() => {
    if (id === null) return;
    const goTo = neighbors.data?.next ?? neighbors.data?.prev ?? null;
    invoke("trash_assets", { ids: [id] })
      .then(() => {
        toast({ message: "Moved to Trash", undo: () => void invoke("restore_assets", { ids: [id] }) });
        dropFromViewerSequence(id);
        if (goTo != null) go(goTo);
        else close();
      })
      .catch((e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not move to Trash." }));
  }, [id, neighbors.data, go, close]);

  // Play (a memory's highlights): one picture every four seconds along the sequence, until
  // the end, a manual move, or Space. In the URL like everything else about the viewer.
  const playing = params.get("play") === "1" && !editing;
  const stopPlaying = useCallback(
    () =>
      setParams(
        (p) => {
          p.delete("play");
          return p;
        },
        { replace: true },
      ),
    [setParams],
  );
  useEffect(() => {
    if (!playing || id === null) return;
    const t = setTimeout(() => {
      const next = neighbors.data?.next;
      if (next != null) go(next, 1);
      else stopPlaying();
    }, PLAY_EVERY_MS);
    return () => clearTimeout(t);
  }, [playing, id, neighbors.data?.next, go, stopPlaying]);
  /** Paging by hand ends a slideshow. */
  const step = useCallback(
    (to: number | null | undefined, direction = 1) => {
      if (playing) stopPlaying();
      go(to, direction);
    },
    [playing, stopPlaying, go],
  );

  // Edit mode is in the URL like the viewer itself, so reload and Back behave.
  const setEditing = useCallback(
    (on: boolean) =>
      setParams((p) => {
        if (on) p.set("edit", "1");
        else p.delete("edit");
        return p;
      }),
    [setParams],
  );

  // Keyboard (UX spec §7). Edit mode has its own keys.
  useEffect(() => {
    if (!open || editing) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      // A text field keeps its keys. The seek bar keeps its arrows and numbers too, but
      // Escape and F11 still reach the viewer, so full screen is always left (Round 8B).
      if (isTyping(t)) return;
      const inField = t instanceof HTMLInputElement;
      const is = (id: string) => matches(e, id);
      if (inField && !is("viewer.close") && !is("viewer.fullscreen")) return;
      const noMod = !e.ctrlKey && !e.metaKey;
      if (is("viewer.close")) {
        e.preventDefault();
        // The text over the photo goes first (Round 13-3), then a box being drawn.
        if (textShown) setShowText(false);
        else if (drawing) setDrawing(false);
        // Fullscreen is left first; a second Escape closes the viewer.
        else if (fullscreen) toggleFullscreen();
        else close();
      } else if (is("viewer.fullscreen")) {
        e.preventDefault();
        toggleFullscreen();
      } else if (is("viewer.previous")) {
        e.preventDefault();
        step(neighbors.data?.prev, -1);
      } else if (is("viewer.next")) {
        e.preventDefault();
        step(neighbors.data?.next, 1);
      } else if (is("viewer.info")) {
        setInfo((v) => !v);
      } else if (is("show-pets")) {
        // People: every face's box over the photo (Round 4I); with Shift, every pet's
        // (Round 8B). F is Favourite.
        if (taggable && noMod) setShowPets((v) => !v);
      } else if (is("tag-people")) {
        if (taggable && noMod) setShowFaces((v) => !v);
      } else if (is("show-text")) {
        // The text read in the photo, over it (Round 13-3).
        if (hasText && noMod) setShowText((v) => !v);
      } else if (is("edit-photo")) {
        if (editable && noMod) {
          e.preventDefault();
          setEditing(true);
        }
      } else if (is("viewer.favorite")) {
        toggleFavorite();
      } else if (is("viewer.trash")) {
        trash();
      } else if (is("viewer.hide-controls")) {
        e.preventDefault();
        setIdle((v) => !v);
      } else if (is("viewer.play-pause")) {
        if (playing) {
          e.preventDefault();
          stopPlaying();
        } else if (live.src) {
          e.preventDefault();
          live.toggle();
        }
      } else if (is("viewer.fit")) {
        e.preventDefault();
        zoom.current?.zoomTo("fit");
      } else if (is("viewer.zoom-1-1")) {
        e.preventDefault();
        zoom.current?.zoomTo("1:1");
      } else if (noMod && !e.altKey && /^[0-5]$/.test(e.key) && (e.key !== "0" || is("viewer.clear-rating"))) {
        // The bare digits are ratings (UX spec §7); the zooms above took Ctrl/Cmd.
        e.preventDefault();
        rate(Number(e.key));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, editing, editable, setEditing, close, step, playing, stopPlaying, neighbors.data, toggleFavorite, trash, rate, live, fullscreen, toggleFullscreen, drawing, taggable, textShown, hasText]);

  const reveal = () => {
    if (id === null) return;
    invoke("reveal_asset", { id }).catch((e: unknown) =>
      toast({ message: isAppError(e) ? e.message : "Could not open the file manager." }),
    );
  };

  const a: AssetDetail | undefined = asset.data;
  // Round 22 lane C: *Edit in* and *Open with ▸* in the photo's menu.
  const openWith = useOpenWith(a ? { mediaType: a.mediaType, raw: a.raw } : null);
  const contributions = useMenuContributions("viewer.menu", navigate);

  // Right-click on the photo (Round 4G): the tile's actions for it, and the viewer's own.
  const photoMenuFor = (p: AssetDetail) => {
    const photo = p.mediaType !== 1 && p.renderable;
    const albumId = gridScope?.scope.albumId ?? null;
    const people = new Map<number, { personId: number; name: string; faceId: number }>();
    for (const f of faces.data ?? []) if (f.personId != null && f.name && !people.has(f.personId)) people.set(f.personId, { personId: f.personId, name: f.name, faceId: f.id });
    return viewerMenu(
      { favorite, rating, mac: isMac(), inHidden: inHiddenFolder(), inAlbum: albumId !== null, people: [...people.values()], openWith: openWith.context, contributions: contributions.map((c) => ({ ...c, run: () => c.run([p.id]) })) },
      {
        favorite: toggleFavorite,
        rate,
        addToAlbum: () => openAlbumPicker([p.id]),
        rotate: (dir) => void rotatePhotos(qc, [p.id], dir),
        copyImage: photo
          ? () =>
              void invoke("copy_image_to_clipboard", { id: p.id }).then(
                () => toast({ message: "Copied the photo" }),
                (e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not copy the photo." }),
              )
          : undefined,
        export: () => openExportSheet([p.id]),
        develop: photo ? () => openDevelop(navigate, viewerSequenceIds().length > 0 ? { only: viewerSequenceIds(), label: "Photos in view" } : {}, p.id) : undefined,
        reveal,
        setAlbumCover:
          albumId !== null
            ? () =>
                void invoke("set_album_cover", { albumId, assetId: p.id }).then(
                  () => {
                    void qc.invalidateQueries({ queryKey: queryKeys.albums });
                    toast({ message: "Album cover updated" });
                  },
                  (e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not set the cover." }),
                )
            : undefined,
        useAsMainPhoto: (person, face) => void peopleActions.setCover(person, face),
        tagPeople: taggable
          ? () => {
              setShowFaces(true);
              setInfo(true);
            }
          : undefined,
        openWith: { editIn: (appId) => openWith.editIn([p.id], appId), chooseApp: () => openWith.chooseApp([p.id]) },
        showInfo: () => setInfo(true),
        moveToHidden: () => void moveToHidden(qc, [p.id]),
        trash,
      },
    );
  };
  // Press and hold the photo to play its motion, as on the phone. A short press is left to
  // the zoom and pan that own the pointer.
  const hold = useRef<ReturnType<typeof setTimeout> | null>(null);
  const holdStart = () => {
    if (!live.src) return;
    hold.current = setTimeout(live.play, 250);
  };
  const holdEnd = () => {
    if (hold.current) clearTimeout(hold.current);
    hold.current = null;
    if (live.playing) live.stop();
  };
  const subtitle = a
    ? [sequence ? `${sequence.index + 1} of ${sequence.total}` : null, a.capturedAt ? formatCaptured(a.capturedAt) : null, a.deviceName]
        .filter(Boolean)
        .join(" · ")
    : "";
  const drag = useWindowDrag();

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="viewer"
          data-part="viewer"
          role="dialog"
          aria-modal="true"
          aria-label={a?.filename ?? "Photo"}
          data-idle={idle && !editing}
          data-info={info && !editing}
          data-editing={editing || undefined}
          data-fullscreen={fullscreen}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
        >
          <div className="viewer__backdrop" onClick={editing ? undefined : close} />
          <WindowChrome autoHide />
          {editing && a ? (
            <EditMode key={a.id} asset={a} fallbackSrc={src} onExit={() => setEditing(false)} />
          ) : (
            <>
              <div
                className="viewer__stage"
                ref={stageRef}
                onContextMenu={a ? (e) => openFaceMenu(e, photoMenuFor(a)) : undefined}
                onPointerDown={live.isLive ? holdStart : undefined}
                onPointerUp={live.isLive ? holdEnd : undefined}
                onPointerCancel={live.isLive ? holdEnd : undefined}
              >
                {a && a.mediaType === 1 ? (
                  <VideoPlayer key={a.id} id={a.id} extension={a.extension} trim={savedRecipe.data?.trim} idle={idle && !editing} />
                ) : a && !a.renderable ? (
                  <div className="viewer__unsupported" role="status">
                    <span>{a.extension.toUpperCase()}</span>
                    <p>This format needs FFmpeg to display. Set it up in Settings → Playback &amp; codecs.</p>
                  </div>
                ) : (
                  <AnimatePresence initial={false} custom={dir} mode="popLayout">
                    <motion.div
                      key={id}
                      className="viewer__image"
                      custom={dir}
                      variants={playing ? FADE : PAGE}
                      initial="enter"
                      animate="centre"
                      exit="leave"
                      transition={playing ? { duration: 0.8, ease: "easeInOut" } : { duration: 0.26, ease: [0.2, 0.8, 0.2, 1] }}
                    >
                      <ZoomPan
                        ref={zoom}
                        src={src}
                        alt={a?.filename ?? ""}
                        natural={a?.width && a?.height ? { width: a.width, height: a.height } : null}
                        onScaleChange={setScale}
                        overlay={
                          hasText || (taggable && (showFaces || drawing || hoverFace !== null || showPets || hoverPet !== null)) ? (
                            <>
                              {hasText && (
                                // Fades in and out as a whole (CSS), never line by line.
                                <div className="viewer__text-layer" data-shown={textShown || undefined} aria-hidden={!textShown}>
                                  <TextOverlay
                                    lines={text.lines}
                                    aspect={shownAspectOf(shownAspect, edited ? savedRecipe.data : null)}
                                    onMenu={(t, e) => openFaceMenu(e, textMenuFor(t))}
                                  />
                                </div>
                              )}
                              {(showFaces || drawing || hoverFace !== null) && (
                                <FaceBoxes
                                  faces={faces.data ?? []}
                                  recipe={edited ? savedRecipe.data : null}
                                  aspect={shownAspect}
                                  show={showFaces}
                                  hovered={hoverFace}
                                  drawing={drawing}
                                  onPick={(face) => setTagging({ face })}
                                  onMenu={(face, e) => openFaceMenu(e, faceMenuFor(face))}
                                  onDrawn={(rect) => {
                                    setDrawing(false);
                                    setTagging({ rect });
                                  }}
                                />
                              )}
                              {(showPets || hoverPet !== null) && (
                                <FaceBoxes
                                  kind="pet"
                                  faces={pets.data ?? []}
                                  recipe={edited ? savedRecipe.data : null}
                                  aspect={shownAspect}
                                  show={showPets}
                                  hovered={hoverPet}
                                  drawing={false}
                                  label={(p) => p.name ?? `Unknown ${p.species}`}
                                  ariaLabel={(p) => (p.name ? `${p.name}: change which pet this is` : `Unknown ${p.species}: tag this pet`)}
                                  onPick={(p) => petBoxMenu.tagBox(p)}
                                  onMenu={(p, e) => openFaceMenu(e, petBoxMenu.menuFor(p))}
                                  onDrawn={() => {}}
                                />
                              )}
                            </>
                          ) : undefined
                        }
                      />
                      <LiveMotionLayer live={live} />
                    </motion.div>
                  </AnimatePresence>
                )}
              </div>

              <div className="viewer__top" data-part="toolbar" {...drag}>
                <button type="button" className="viewer__icon" aria-label="Close" onClick={close}>
                  <IconX size={18} />
                </button>
                <div className="viewer__meta">
                  <span className="viewer__title-row">
                    <span className="viewer__name">{a?.filename ?? "…"}</span>
                    {a?.raw && (
                      <span className="viewer__tag" title="Camera RAW original">
                        RAW
                      </span>
                    )}
                    {a && a.hdr > 0 && (
                      <span className="viewer__tag" title="High dynamic range">
                        HDR
                      </span>
                    )}
                    <LiveButton live={live} />
                  </span>
                  <span className="viewer__sub">{subtitle}</span>
                </div>
                <div className="viewer__spacer" />
                <ToolButton label={favorite ? "Remove from Favorites" : "Add to Favorites"} shortcut="f" pressed={favorite} onClick={toggleFavorite}>
                  {favorite ? <IconStarFilled size={18} className="viewer__star" /> : <IconStar size={18} />}
                </ToolButton>
                <ToolButton label="Info" shortcut="i" pressed={info} onClick={() => setInfo((v) => !v)}>
                  <IconInfoCircle size={18} />
                </ToolButton>
                {hasText && (
                  <ToolButton label="Text" shortcut="t" pressed={textShown} onClick={() => setShowText((v) => !v)}>
                    <IconTextScan2 size={18} />
                  </ToolButton>
                )}
                {editable && (
                  <ToolButton label={a?.mediaType === 1 ? "Trim" : "Edit"} shortcut="e" onClick={() => setEditing(true)}>
                    <IconAdjustmentsHorizontal size={18} />
                  </ToolButton>
                )}
                {a && a.mediaType !== 1 && a.renderable && (
                  <ToolButton
                    label="Develop"
                    onClick={() => {
                      // The set the viewer was opened on (the Map's), or the whole library.
                      const set = viewerSequenceIds();
                      openDevelop(navigate, set.length > 0 ? { only: set, label: "Photos in view" } : {}, a.id);
                    }}
                  >
                    <IconAperture size={18} />
                  </ToolButton>
                )}
                {a && (
              <ToolButton label="Export…" onClick={() => openExportSheet([a.id])}>
                <IconFileExport size={18} />
              </ToolButton>
            )}
            <ToolButton label="Show file" onClick={reveal}>
                  <IconFolderOpen size={18} />
                </ToolButton>
                <span className="viewer__rating">
                  <StarRating value={rating} onChange={rate} size={17} />
                </span>
                <ToolButton
                  label={scale > 1.01 ? "Fit to window" : "Actual size"}
                  shortcut={scale > 1.01 ? "Ctrl 0" : "Ctrl 1"}
                  onClick={() => zoom.current?.zoomTo(scale > 1.01 ? "fit" : "1:1")}
                >
                  {scale > 1.01 ? <IconArrowsMaximize size={18} /> : <IconZoomIn size={18} />}
                </ToolButton>
                <ToolButton label={fullscreen ? "Exit full screen" : "Full screen"} shortcut="F11" pressed={fullscreen} onClick={toggleFullscreen}>
                  {fullscreen ? <IconMinimize size={18} /> : <IconMaximize size={18} />}
                </ToolButton>
                <ToolButton label="Move to Trash" shortcut="Delete" danger onClick={trash}>
                  <IconTrash size={18} />
                </ToolButton>
              </div>
              {loading && (
                <div className="viewer__loading">
                  <Spinner label="Loading" />
                  {stage === "thumb" ? "Loading preview…" : oversized ? "Loading full resolution…" : "Loading full quality…"}
                </div>
              )}
              {shown !== null && (
                <div className="viewer__loading" role="status">
                  Shown at {shown} % — larger than this PC can open in full
                </div>
              )}

              <button
                type="button"
                className="viewer__nav viewer__nav--prev"
                aria-label="Previous"
                disabled={!neighbors.data?.prev}
                onClick={() => step(neighbors.data?.prev, -1)}
              >
                <IconChevronLeft size={22} />
              </button>
              <button
                type="button"
                className="viewer__nav viewer__nav--next"
                aria-label="Next"
                disabled={!neighbors.data?.next}
                onClick={() => step(neighbors.data?.next, 1)}
              >
                <IconChevronRight size={22} />
              </button>

              <div className="viewer__strip" aria-hidden>
                {[neighbors.data?.prev, id, neighbors.data?.next].map((n, i) =>
                  n != null ? (
                    <img key={`${i}-${n}`} className="viewer__strip-item" data-current={n === id} src={mediaUrl("thumb", n, { s: 256 })} alt="" onClick={() => step(n)} />
                  ) : (
                    <span key={`${i}-empty`} className="viewer__strip-item viewer__strip-item--empty" />
                  ),
                )}
              </div>

              <AnimatePresence>
                {info && a && (
                  <motion.div
                    className="viewer__panel"
                    initial={{ x: 40, opacity: 0 }}
                    animate={{ x: 0, opacity: 1 }}
                    exit={{ x: 40, opacity: 0 }}
                    transition={{ type: "spring", stiffness: 420, damping: 36 }}
                  >
                    <InfoPanel
                      asset={a}
                      onReveal={reveal}
                      onOpen={(id) => go(id)}
                      faceTools={taggable ? faceTools : undefined}
                      petTools={taggable ? petTools : undefined}
                      text={text.on && taggable ? { lines: text.lines, reading: text.reading, shown: textShown, onShowOnPhoto: () => setShowText((v) => !v), onMenu: (t, e) => openFaceMenu(e, textMenuFor(t)) } : undefined}
                    />
                  </motion.div>
                )}
              </AnimatePresence>
              {petBoxMenu.picker}
              {tagging && (
                <PersonPicker
                  title="Who is this?"
                  people={people}
                  exclude={"face" in tagging ? (tagging.face.personId ?? undefined) : undefined}
                  onPick={(person) => tagAs({ person })}
                  onCreate={(newName) => tagAs({ newName })}
                  onClose={() => setTagging(null)}
                />
              )}
            </>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
