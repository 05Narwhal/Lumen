import { Button, TextInput } from "@mantine/core";
import { listen } from "@tauri-apps/api/event";
import { IconMapPin, IconMapPinOff } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { Segmented } from "@/components/segmented";
import { toast } from "@/components/toast";
import { usePlugins } from "@/hooks/use-plugins";
import { useSettings } from "@/hooks/use-settings";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { placeReviewOpen, placeReviewRequests, type PlaceReviewAction } from "@/lib/place-review";
import { searchPlaces, type PlaceResult } from "@/lib/place-search";
import { isTyping } from "@/lib/typing";
import type { ReviewAction } from "@/types/generated/ReviewAction";
import type { ReviewState } from "@/types/generated/ReviewState";
import type { UndoReport } from "@/types/generated/UndoReport";
import type { WriteFinished } from "@/types/generated/WriteFinished";
import { mediaUrl } from "@/utils/media";
import { confirmOf, FLAG_TEXT, itemsOf, moveOf, pinsOf, rejectOf, STATE_TEXT, step, tabView, targets, type Pin, type ReviewTab } from "./model";
import { PlaceMap } from "./place-map";
import "./styles.scss";

const photos = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);
/** Show all stops drawing thumbnails here; a moment can hold thousands. */
const SHOW_ALL_LIMIT = 120;

type Mode = "none" | "type" | "pin";

/**
 * `/review/places?plugin=<id>` (Round 24 W4, spec §8.6): the places a plugin guessed, to check
 * before anything is written. Nothing here knows which plugin it is. Every action is sent to
 * the plugin (`places_review_event`), which returns the suggestions again; only *Write* commits.
 */
export function PlaceReview() {
  const [params] = useSearchParams();
  const pluginId = params.get("plugin") ?? "";
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { settings } = useSettings();
  const { plugins } = usePlugins();
  const pluginName = plugins.find((p) => p.id === pluginId)?.name ?? "";

  const key = useMemo(() => ["places-review", pluginId] as const, [pluginId]);
  const review = useQuery({
    queryKey: key,
    queryFn: () => invoke<ReviewState>("places_review", { pluginId }),
    enabled: pluginId !== "",
    retry: false,
    staleTime: Infinity,
  });
  const state = review.data;

  const [tab, setTab] = useState<ReviewTab>("check");
  const chose = useRef(false);
  const [selectedGroup, setSelectedGroup] = useState<string | null>(null);
  const [selectedAsset, setSelectedAsset] = useState<number | null>(null);
  const [showAll, setShowAll] = useState(false);
  const [mode, setMode] = useState<Mode>("none");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<PlaceResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [writing, setWriting] = useState(false);
  const [lastWrite, setLastWrite] = useState<WriteFinished["report"] | null>(null);

  // With nothing to check, open on the moments rather than an empty tab.
  useEffect(() => {
    if (state && !chose.current && state.counts.check === 0) setTab("moments");
  }, [state]);

  const view = useMemo(() => (state ? tabView(state, tab) : { items: [], moments: [] }), [state, tab]);
  const moment = view.moments.find((g) => g.id === selectedGroup) ?? view.moments[0] ?? null;
  const momentItems = useMemo(() => (moment ? itemsOf(view.items, moment.id) : []), [view.items, moment]);
  const pins = useMemo(() => pinsOf(view.items), [view.items]);
  const asset = selectedAsset != null && momentItems.some((i) => i.asset === selectedAsset) ? selectedAsset : null;

  const choose = (t: ReviewTab) => {
    chose.current = true;
    setTab(t);
    setSelectedAsset(null);
    setShowAll(false);
  };
  const selectMoment = (id: string | null) => {
    setSelectedGroup(id);
    setSelectedAsset(null);
    setShowAll(false);
    setMode("none");
  };

  const send = useCallback(
    async (event: ReviewAction, advanceTo?: string | null) => {
      if (busy) return;
      setBusy(true);
      try {
        const next = await invoke<ReviewState>("places_review_event", { pluginId, event });
        qc.setQueryData(key, next);
        if (advanceTo !== undefined) setSelectedGroup(advanceTo);
        setSelectedAsset(null);
      } catch (e) {
        toast({ message: isAppError(e) ? e.message : "The plugin couldn't take that." });
      } finally {
        setBusy(false);
      }
    },
    [busy, key, pluginId, qc],
  );

  const neighbour = () => {
    const at = view.moments.findIndex((g) => g.id === moment?.id);
    return (view.moments[at + 1] ?? view.moments[at - 1])?.id ?? null;
  };
  const scope = (kind: "confirm" | "reject") => (asset != null ? [asset] : targets(momentItems, kind));

  const confirm = () => {
    const ids = scope("confirm");
    if (ids.length) void send(confirmOf(ids), asset != null ? undefined : neighbour());
  };
  const reject = () => {
    const ids = scope("reject");
    if (ids.length) void send(rejectOf(ids), asset != null ? undefined : neighbour());
  };
  const split = () => {
    if (!moment) return;
    if (asset == null) {
      setShowAll(true);
      toast({ message: "Pick the photo the new moment starts at, then split." });
      return;
    }
    void send({ kind: "split", group: moment.id, atAsset: asset });
  };
  const move = (lat: number, lon: number) => {
    const ids = scope("reject");
    setMode("none");
    if (ids.length) void send(moveOf(ids, lat, lon));
  };
  const go = (delta: 1 | -1) => {
    const next = step(view.moments, moment, delta);
    if (next) selectMoment(next.id);
  };

  // Typing a place: the Map page's search (bundled atlas, online when its setting is on).
  const online = settings?.map.online ?? true;
  useEffect(() => {
    if (mode !== "type" || query.trim().length < 2) return setHits([]);
    let alive = true;
    const t = setTimeout(() => void searchPlaces(query.trim(), { online, limit: 6 }).then((r) => alive && setHits(r)), 250);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [mode, query, online]);

  // Write: a background job (activity ring); the result arrives as an event.
  const write = useCallback(async () => {
    if (writing) return;
    setWriting(true);
    try {
      await invoke("places_write", { pluginId });
    } catch (e) {
      setWriting(false);
      toast({ message: isAppError(e) ? e.message : "Couldn't start writing." });
    }
  }, [pluginId, writing]);

  const stateRef = useRef({ qc, key });
  stateRef.current = { qc, key };
  useEffect(() => {
    const un = listen<WriteFinished>("places://write-finished", (e) => {
      if (e.payload.pluginId !== pluginId) return;
      const r = e.payload.report;
      setWriting(false);
      setLastWrite(r);
      void invalidateLibraryData(stateRef.current.qc);
      void stateRef.current.qc.invalidateQueries({ queryKey: stateRef.current.key });
      toast({
        message: r.cancelled ? `Stopped after ${r.locations.toLocaleString()} locations` : `${r.locations.toLocaleString()} locations written`,
        undo: () => void undo(r.writeId),
        durationMs: 6000,
      });
    });
    return () => void un.then((f) => f()).catch(() => {});
    // `undo` only reads refs and the stable query client.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId]);

  async function undo(writeId: number) {
    try {
      const r = await invoke<UndoReport>("places_undo", { writeId });
      setLastWrite(null);
      void invalidateLibraryData(stateRef.current.qc);
      void stateRef.current.qc.invalidateQueries({ queryKey: stateRef.current.key });
      toast({ message: r.keptInFile.length ? `Undone. ${r.keptInFile.length === 1 ? "1 file kept" : `${r.keptInFile.length} files kept`} its location.` : "Undone" });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Couldn't undo that." });
    }
  }

  // Keys.
  const act = useRef<(a: PlaceReviewAction) => void>(() => {});
  act.current = (a) => {
    if (a === "confirm") confirm();
    else if (a === "reject") reject();
    else if (a === "split") split();
    else if (a === "next") go(1);
    else if (a === "previous") go(-1);
    else if (a === "move-place") setMode((m) => (m === "type" ? "none" : "type"));
    else if (a === "move-pin") setMode((m) => (m === "pin" ? "none" : "pin"));
    else if (a === "write") void write();
  };
  useEffect(() => {
    placeReviewOpen.set(true);
    return () => placeReviewOpen.set(false);
  }, []);
  const seen = useRef(placeReviewRequests.get().n);
  const request = placeReviewRequests.use();
  useEffect(() => {
    if (request.n === seen.current) return;
    seen.current = request.n;
    if (request.action) act.current(request.action);
  }, [request]);
  useEffect(() => {
    const keys: Record<string, PlaceReviewAction> = { Enter: "confirm", r: "reject", m: "move-place", s: "split", ArrowRight: "next", ArrowLeft: "previous" };
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target)) return;
      const a = keys[e.key.length === 1 ? e.key.toLowerCase() : e.key];
      if (!a) return;
      // A focused button or link takes Enter itself.
      if (e.key === "Enter" && e.target instanceof HTMLElement && e.target.closest("button, a, [role=tab]")) return;
      e.preventDefault();
      act.current(a);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pinSelect = (p: Pin) => {
    setSelectedGroup(p.group);
    setSelectedAsset(p.assets.length === 1 ? p.assets[0] : null);
  };

  const header = <PageHeader title={pluginName ? `Place review · ${pluginName}` : "Place review"} search={false} back={{ to: "/", label: "Back" }} />;

  if (pluginId === "" || review.isError || (state && state.counts.check + state.counts.moments + state.counts.done === 0)) {
    return (
      <div className="review-places">
        {header}
        <EmptyState
          icon={<IconMapPinOff size={40} />}
          title="Nothing to review."
          description={review.isError && isAppError(review.error) ? review.error.message : "A plugin that guesses where photos were taken will show them here."}
          action={
            <Button variant="light" onClick={() => navigate(-1)}>
              Back
            </Button>
          }
        />
      </div>
    );
  }
  if (!state) return <div className="review-places">{header}</div>;

  const c = state.counts;
  const total = view.moments.length;
  const index = moment ? view.moments.findIndex((g) => g.id === moment.id) + 1 : 0;
  const toCheck = moment ? momentItems.filter((i) => i.status === "pending" && (i.state === "guess" || i.flag !== null)).length : 0;
  const shown = showAll ? momentItems.slice(0, SHOW_ALL_LIMIT) : momentItems.slice(0, 3);

  return (
    <div className="review-places">
      {header}
      <div className="review-places__bar">
        <Segmented
          label="Review"
          value={tab}
          onChange={choose}
          data={[
            { value: "check", label: `Check these ${c.check.toLocaleString()}` },
            { value: "moments", label: `Moments ${c.moments.toLocaleString()}` },
            { value: "done", label: `Done ${c.done.toLocaleString()}` },
          ]}
        />
        <Button onClick={() => void write()} loading={writing} disabled={c.done === 0} title="Put these places in your library, and in the files where the plugin may">
          Write {c.done.toLocaleString()} {c.done === 1 ? "location" : "locations"}
        </Button>
      </div>
      <div className="review-places__body">
        <div className="review-places__map">
          <PlaceMap
            pins={pins}
            selectedGroup={moment?.id ?? null}
            online={online}
            style={settings?.map.style ?? "map"}
            esriKey={settings?.map.esriKey}
            placing={mode === "pin"}
            onSelectPin={pinSelect}
            onMovePin={(p, lat, lon) => void send(moveOf(p.assets, lat, lon))}
            onPlace={move}
          />
          <p className="review-places__legend" aria-hidden>
            <span data-state="anchor" /> known <span data-state="inferred" /> worked out <span data-state="guess" /> guess · drag a pin to move it
          </p>
        </div>
        <section className="review-places__card" aria-label="Moment">
          {moment ? (
            <>
              <h2 className="review-places__title">{moment.label || "Moment"}</h2>
              <p className="review-places__sub">
                {photos(momentItems.length)}
                {toCheck > 0 && ` · ${toCheck.toLocaleString()} to check`} · {index} of {total}
              </p>
              <div className="review-places__thumbs" data-all={showAll}>
                {shown.map((i) => (
                  <button
                    key={i.asset}
                    type="button"
                    className="review-places__thumb"
                    data-selected={i.asset === asset}
                    aria-pressed={i.asset === asset}
                    aria-label={`Photo ${i.asset}${i.flag ? `: ${FLAG_TEXT[i.flag]}` : ""}`}
                    title={[STATE_TEXT[i.state], i.flag ? FLAG_TEXT[i.flag] : null, i.candidates[0]?.label].filter(Boolean).join(" · ")}
                    onClick={() => setSelectedAsset(i.asset === asset ? null : i.asset)}
                  >
                    <img src={mediaUrl("thumb", i.asset, { s: 256 })} alt="" loading="lazy" decoding="async" draggable={false} />
                    <span className="review-places__mark" data-state={i.state} data-flagged={i.flag !== null} aria-hidden />
                  </button>
                ))}
              </div>
              {showAll && momentItems.length > SHOW_ALL_LIMIT && <p className="review-places__more">and {(momentItems.length - SHOW_ALL_LIMIT).toLocaleString()} more</p>}
              {momentItems[0]?.candidates[0]?.label && !showAll && <p className="review-places__place">{momentItems[0].candidates[0].label}</p>}
              <div className="review-places__actions">
                <Button onClick={confirm} disabled={busy || scope("confirm").length === 0}>
                  {asset != null ? "Confirm photo" : "Confirm all"} <kbd>↵</kbd>
                </Button>
                <Button variant="default" onClick={reject} disabled={busy}>
                  Reject <kbd>R</kbd>
                </Button>
                <Button variant="default" onClick={() => setMode(mode === "pin" ? "none" : "pin")} data-active={mode === "pin"}>
                  {mode === "pin" ? "Click the map…" : "Move pin"}
                </Button>
                <Button variant="default" onClick={() => setMode(mode === "type" ? "none" : "type")} data-active={mode === "type"}>
                  Type a place <kbd>M</kbd>
                </Button>
                <Button variant="default" onClick={split} disabled={busy}>
                  Split <kbd>S</kbd>
                </Button>
                <Button variant="subtle" onClick={() => setShowAll(!showAll)}>
                  {showAll ? "Show fewer" : `Show all ${momentItems.length.toLocaleString()}`}
                </Button>
              </div>
              {mode === "type" && (
                <div className="review-places__find">
                  <TextInput
                    autoFocus
                    size="sm"
                    aria-label="Type a place"
                    placeholder="A town, a street, a landmark"
                    value={query}
                    onChange={(e) => setQuery(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") setMode("none");
                      if (e.key === "Enter" && hits[0]) move(hits[0].lat, hits[0].lon);
                    }}
                  />
                  <ul>
                    {hits.map((h) => (
                      <li key={`${h.label}|${h.lat}|${h.lon}`}>
                        <button type="button" onClick={() => move(h.lat, h.lon)}>
                          <IconMapPin size={16} aria-hidden /> {h.label}
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          ) : (
            <p className="review-places__none">Nothing in this tab.</p>
          )}
        </section>
      </div>
      <nav className="review-places__strip" aria-label="Moments in time order">
        {view.moments.map((g) => (
          <button key={g.id} type="button" className="review-places__moment" aria-current={g.id === moment?.id} onClick={() => selectMoment(g.id)} title={g.label}>
            <span className="review-places__moment-label">{g.label || "Moment"}</span>
            <span className="review-places__moment-count">{g.count.toLocaleString()}</span>
            {g.check > 0 && <span className="review-places__moment-check" aria-label={`${g.check} to check`} />}
          </button>
        ))}
      </nav>
      {lastWrite && (lastWrite.lumenOnly > 0 || lastWrite.refused.length > 0 || lastWrite.skipped > 0) && (
        <section className="review-places__result" aria-label="Last write">
          <p>
            {lastWrite.files.toLocaleString()} of {lastWrite.locations.toLocaleString()} also went into the files.
            {lastWrite.lumenOnly > 0 && ` ${lastWrite.lumenOnly.toLocaleString()} saved in Lumen only (a format Lumen doesn't write to).`}
            {lastWrite.skipped > 0 && ` ${lastWrite.skipped.toLocaleString()} skipped: they have a location of their own now.`}
          </p>
          {lastWrite.refused.length > 0 && (
            <ul>
              {lastWrite.refused.slice(0, 20).map(([id, why]) => (
                <li key={id}>
                  <Link to={`?plugin=${encodeURIComponent(pluginId)}&asset=${id}`}>Photo {id}</Link>: {why}
                </li>
              ))}
              {lastWrite.refused.length > 20 && <li>and {(lastWrite.refused.length - 20).toLocaleString()} more</li>}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}
