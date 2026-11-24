import { Button, Select, TextInput } from "@mantine/core";
import { IconListCheck } from "@tabler/icons-react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { toast } from "@/components/toast";
import { usePlugins } from "@/hooks/use-plugins";
import { useSettings } from "@/hooks/use-settings";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { describeValue, editText, KIND_WORDS, LABEL_COLOURS, valueFromText, type ProposalKind } from "@/lib/proposals";
import type { AcceptReport } from "@/types/generated/AcceptReport";
import type { AcceptUndo } from "@/types/generated/AcceptUndo";
import type { ProposalReviewItem } from "@/types/generated/ProposalReviewItem";
import type { ProposalReviewPage } from "@/types/generated/ProposalReviewPage";
import type { ProposalValue } from "@/types/generated/ProposalValue";
import { mediaUrl } from "@/utils/media";
import type { Pin } from "../places/model";
import { PlaceMap } from "../places/place-map";
import "./styles.scss";

const BARS = [0.5, 0.7, 0.8, 0.9, 0.95];
const pct = (c: number) => `${Math.round(c * 100)}%`;
const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const many = (n: number, noun: string) => `${n} ${noun}${n === 1 ? "" : "s"}`;

/** One proposal to accept; `value` is what the person changed it to. */
interface Take {
  id: number;
  value?: ProposalValue;
}

/**
 * `/review/<kind>?plugin=<id>` for a kind plugins submit through `proposals.submit`: a place, a
 * date, tags, a colour label, a name, a title or a caption. Each row shows the photo, what it has
 * now and what the plugin proposes; *Accept*, *Edit* and *Reject* act on one, *Accept all* on
 * those above a confidence. Accepting writes through the app's own paths to Lumen's records
 * only; every action can be undone from its toast. Nothing here knows which plugin it is.
 */
export function ProposalReview({ kind }: { kind: ProposalKind }) {
  const [params] = useSearchParams();
  const pluginId = params.get("plugin") || null;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { settings } = useSettings();
  const { plugins } = usePlugins();
  const pluginName = plugins.find((p) => p.id === pluginId)?.name ?? "";
  const words = KIND_WORDS[kind];

  const key = useMemo(() => ["proposals", kind, pluginId] as const, [kind, pluginId]);
  const list = useInfiniteQuery({
    queryKey: key,
    queryFn: ({ pageParam }) => invoke<ProposalReviewPage>("proposals_list", { kind, pluginId, cursor: pageParam }),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.nextCursor,
    staleTime: Infinity,
  });
  const rows = useMemo(() => list.data?.pages.flatMap((p) => p.items) ?? [], [list.data]);

  const [bar, setBar] = useState(0.9);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  /** A pin the person dragged: that row is accepted with the place where it was dropped. */
  const [moved, setMoved] = useState<Record<number, ProposalValue>>({});

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["proposals"] });
    void invalidateLibraryData(qc);
  };
  const failed = (e: unknown) => toast({ message: isAppError(e) ? e.message : "Couldn't do that." });

  async function undoAccept(undo: AcceptUndo) {
    try {
      await invoke("proposals_undo_accept", { undo });
      refresh();
    } catch (e) {
      failed(e);
    }
  }

  async function accept(items: Take[]) {
    if (busy || items.length === 0) return;
    setBusy(true);
    try {
      const r = await invoke<AcceptReport>("proposals_accept", { items });
      setEditing(null);
      refresh();
      if (r.accepted.length) {
        toast({ message: `Accepted ${many(r.accepted.length, words.noun)}.`, undo: () => void undoAccept(r.undo) });
      }
      if (r.skipped.length) {
        const more = r.skipped.length > 1 ? ` (and ${r.skipped.length - 1} more)` : "";
        toast({ message: `${r.skipped[0].reason}${more}` });
      }
    } catch (e) {
      failed(e);
    } finally {
      setBusy(false);
    }
  }

  async function reject(ids: number[]) {
    try {
      await invoke("proposals_reject", { ids });
      refresh();
      toast({
        message: `Rejected ${many(ids.length, words.noun)}.`,
        undo: () => void invoke("proposals_restore", { ids }).then(refresh, failed),
      });
    } catch (e) {
      failed(e);
    }
  }

  const header = <PageHeader title={pluginName ? `${words.title} · ${pluginName}` : words.title} search={false} back={{ to: "/", label: "Back" }} />;
  const takeOf = (r: ProposalReviewItem): Take => (moved[r.proposal.id] ? { id: r.proposal.id, value: moved[r.proposal.id] } : { id: r.proposal.id });

  if (list.isSuccess && rows.length === 0) {
    return (
      <div className="proposal-review">
        {header}
        <EmptyState
          icon={<IconListCheck size={40} />}
          title="Nothing to review."
          description="When a plugin proposes something for your photos, it waits here until you accept it."
          action={
            <Button variant="light" onClick={() => navigate(-1)}>
              Back
            </Button>
          }
        />
      </div>
    );
  }

  const above = rows.filter((r) => r.proposal.confidence >= bar);
  const mapped = kind === "location";
  const focus = selected ?? rows[0]?.proposal.id ?? null;
  const pins: Pin[] = mapped
    ? rows.flatMap((r) => {
        const v = moved[r.proposal.id] ?? r.proposal.proposed;
        return v.kind === "location"
          ? [
              {
                key: String(r.proposal.id),
                lat: v.value.lat,
                lon: v.value.lon,
                state: r.proposal.id === focus ? ("anchor" as const) : ("guess" as const),
                assets: [r.proposal.assetId],
                group: String(r.proposal.id),
                confirmed: false,
              },
            ]
          : [];
      })
    : [];

  return (
    <div className="proposal-review">
      {header}
      <div className="proposal-review__bar">
        <Select
          aria-label="Confidence for Accept all"
          size="xs"
          w={130}
          allowDeselect={false}
          data={BARS.map((b) => ({ value: String(b), label: `Above ${pct(b)}` }))}
          value={String(bar)}
          onChange={(v) => v && setBar(Number(v))}
        />
        <Button size="xs" loading={busy} disabled={above.length === 0} onClick={() => void accept(above.map(takeOf))}>
          {`Accept ${above.length} above ${pct(bar)}`}
        </Button>
      </div>
      <div className="proposal-review__body" data-map={mapped}>
        <ul className="proposal-review__list">
          {rows.map((r) => (
            <Row
              key={r.proposal.id}
              item={r}
              kind={kind}
              shown={moved[r.proposal.id] ?? r.proposal.proposed}
              focused={mapped && focus === r.proposal.id}
              editing={editing === r.proposal.id}
              onFocus={() => setSelected(r.proposal.id)}
              onEdit={(on) => setEditing(on ? r.proposal.id : null)}
              onAccept={(value) => void accept([value ? { id: r.proposal.id, value } : takeOf(r)])}
              onReject={() => void reject([r.proposal.id])}
            />
          ))}
          {list.hasNextPage && (
            <li className="proposal-review__more">
              <Button variant="subtle" size="xs" loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()}>
                Show more
              </Button>
            </li>
          )}
        </ul>
        {mapped && (
          <div className="proposal-review__map" data-testid="proposal-map">
            <PlaceMap
              pins={pins}
              selectedGroup={focus === null ? null : String(focus)}
              online={settings?.map.online ?? true}
              style={settings?.map.style ?? "map"}
              esriKey={settings?.map.esriKey}
              placing={false}
              onSelectPin={(p) => setSelected(Number(p.group))}
              onMovePin={(p, lat, lon) => setMoved((m) => ({ ...m, [Number(p.group)]: { kind: "location", value: { lat, lon } } }))}
              onPlace={() => {}}
            />
            <p className="proposal-review__legend" aria-hidden>
              Drag a pin to change the place before you accept it
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

interface RowProps {
  item: ProposalReviewItem;
  kind: ProposalKind;
  shown: ProposalValue;
  focused: boolean;
  editing: boolean;
  onFocus: () => void;
  onEdit: (on: boolean) => void;
  onAccept: (value?: ProposalValue) => void;
  onReject: () => void;
}

function Row({ item, kind, shown, focused, editing, onFocus, onEdit, onAccept, onReject }: RowProps) {
  const { proposal, filename, current } = item;
  const [text, setText] = useState("");
  const edited = editing ? valueFromText(kind, text, proposal.proposed) : null;
  const label = `${capital(KIND_WORDS[kind].noun)} for ${filename}`;
  const face = proposal.proposed.kind === "person" ? proposal.proposed.value.face : null;

  return (
    <li className="proposal-review__row" data-focused={focused} onClick={onFocus}>
      <div className="proposal-review__thumb">
        {face !== null ? <img src={mediaUrl("face", face)} alt="The face" loading="lazy" /> : <img src={mediaUrl("thumb", proposal.assetId)} alt="" loading="lazy" />}
      </div>
      <div className="proposal-review__what">
        <p className="proposal-review__file">{filename}</p>
        <dl className="proposal-review__values">
          <dt>Now</dt>
          <dd data-empty={!current}>{current ? describeValue(current) : "Nothing yet"}</dd>
          <dt>Proposed</dt>
          <dd data-proposed>{describeValue(shown)}</dd>
        </dl>
        {proposal.reason && <p className="proposal-review__reason">{proposal.reason}</p>}
        {editing && (
          <div className="proposal-review__edit">
            {kind === "label" ? (
              <Select aria-label={label} size="xs" allowDeselect={false} data={[...LABEL_COLOURS]} value={text || null} onChange={(v) => setText(v ?? "")} />
            ) : (
              <TextInput
                aria-label={label}
                size="xs"
                value={text}
                placeholder={kind === "date" ? "YYYY-MM-DDTHH:MM:SS" : kind === "location" ? "latitude, longitude" : kind === "tags" ? "tag, tag" : undefined}
                onChange={(e) => setText(e.currentTarget.value)}
              />
            )}
            <Button size="xs" disabled={!edited} onClick={() => edited && onAccept(edited)}>
              Save and accept
            </Button>
            <Button size="xs" variant="subtle" onClick={() => onEdit(false)}>
              Cancel
            </Button>
          </div>
        )}
      </div>
      <span className="proposal-review__confidence" title="How sure the plugin is">
        {pct(proposal.confidence)}
      </span>
      <div className="proposal-review__actions">
        <Button size="xs" aria-label={`Accept ${filename}`} onClick={() => onAccept()}>
          Accept
        </Button>
        <Button
          size="xs"
          variant="light"
          aria-label={`Edit ${filename}`}
          onClick={() => {
            setText(editText(shown));
            onEdit(true);
          }}
        >
          Edit
        </Button>
        <Button size="xs" variant="subtle" color="gray" aria-label={`Reject ${filename}`} onClick={onReject}>
          Reject
        </Button>
      </div>
    </li>
  );
}
