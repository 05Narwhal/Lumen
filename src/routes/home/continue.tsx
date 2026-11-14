import { Button } from "@mantine/core";
import { IconAlbum } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearchParams } from "react-router";
import { CoverStack, photoItems } from "@/components/cover-stack";
import { IconCover } from "@/components/icon-cover";
import { useAlbums } from "@/hooks/use-albums";
import { useHomeSummary } from "@/hooks/use-home";
import { BATCH_COMPLETE, useImport } from "@/hooks/use-import";
import { invoke } from "@/lib/ipc";
import { developSourceStore, isEmptySource, openDevelop } from "@/lib/develop-source";
import { queryKeys } from "@/lib/query-keys";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { ImportBatchSummary } from "@/types/generated/ImportBatchSummary";
import { formatRelative } from "@/utils/date";
import { mediaUrl } from "@/utils/media";

/** Six thumbnails in a row; each one is a button. */
function Thumbs({ ids, label, onPick }: { ids: number[]; label: string; onPick: (id: number) => void }) {
  return (
    <div className="home-route__thumbs">
      {ids.map((id) => (
        <button key={id} type="button" className="home-route__thumb" aria-label={`${label} ${id}`} onClick={() => onPick(id)}>
          <img src={mediaUrl("thumb", id, { s: 256 })} alt="" loading="lazy" decoding="async" />
        </button>
      ))}
    </div>
  );
}

/**
 * What Develop's filmstrip last held (Round 21 §13): a click opens that set at the photo picked.
 * With nothing remembered the card shows the photos with Develop work, and opens all of them.
 */
function InDevelop({ ids, count, label }: { ids: number[]; count: number; label: string | null }) {
  const navigate = useNavigate();
  const open = async (id: number) => {
    const remembered = developSourceStore.get();
    if (label !== null && !isEmptySource(remembered)) {
      openDevelop(navigate, { scope: remembered.scope, only: remembered.only, label: remembered.label }, id);
      return;
    }
    const all = await invoke<number[]>("list_draft_ids").catch(() => ids);
    openDevelop(navigate, { only: all.length > 0 ? all : ids, label: "In Develop" }, id);
  };
  return (
    <article className="home-route__card rise-in" aria-label="In Develop">
      <header className="home-route__card-head">
        <div>
          <h3>In Develop · {count.toLocaleString()}</h3>
          {label && <span className="home-route__card-sub">{label}</span>}
        </div>
        <Button size="xs" variant="subtle" onClick={() => void open(ids[0])}>
          Open Develop
        </Button>
      </header>
      <Thumbs ids={ids} label="Develop photo" onPick={(id) => void open(id)} />
    </article>
  );
}

function LatestImport({ batch }: { batch: ImportBatchSummary }) {
  const [, setParams] = useSearchParams();
  const ids = useQuery({
    queryKey: queryKeys.recent(batch.id),
    queryFn: () => invoke<number[]>("get_recent_asset_ids", { batchId: batch.id, limit: 12 }),
  });
  return (
    <article className="home-route__card rise-in" aria-label="Latest import">
      <header className="home-route__card-head">
        <div>
          <h3>{batch.sourceLabel}</h3>
          <span className="home-route__card-sub">
            {batch.itemCount.toLocaleString()} items · {formatRelative(batch.startedAt)}
          </span>
        </div>
        <Button size="xs" variant="subtle" component={Link} to="/library">
          Open in Library
        </Button>
      </header>
      <Thumbs
        ids={(ids.data ?? []).slice(0, 6)}
        label="Open item"
        onPick={(id) =>
          setParams((p) => {
            p.set("asset", String(id));
            return p;
          })
        }
      />
    </article>
  );
}

function RecentAlbums({ albums }: { albums: AlbumSummary[] }) {
  return (
    <article className="home-route__card rise-in" aria-label="Recent albums">
      <header className="home-route__card-head">
        <h3>Recent albums</h3>
        <Button size="xs" variant="subtle" component={Link} to="/albums">
          All albums
        </Button>
      </header>
      <div className="home-route__albums">
        {albums.map((a) => (
          <Link key={a.id} to={`/albums/${a.id}`} className="home-route__album">
            <span className="home-route__album-cover" data-look={a.coverStyle ? "icon" : a.previewIds.length > 0 ? "prints" : undefined}>
              {a.coverStyle ? <IconCover style={a.coverStyle} /> : a.previewIds.length > 0 ? <CoverStack items={photoItems(a.previewIds)} /> : <IconAlbum size={24} />}
            </span>
            <span className="home-route__album-name">{a.name}</span>
            <span className="home-route__card-sub">{a.itemCount === 1 ? "1 photo" : `${a.itemCount.toLocaleString()} photos`}</span>
          </Link>
        ))}
      </div>
    </article>
  );
}

/**
 * Pick up where you left off (Round 20 §10.3): what is in Develop, the newest import, the
 * albums changed last. A card with nothing to show is not drawn; with none, neither is the row.
 */
export function ContinueRow() {
  const { summary } = useHomeSummary();
  const { batches } = useImport();
  const { albums } = useAlbums();
  const drafts = summary?.drafts ?? [];
  const latest = batches.find((b) => b.state === BATCH_COMPLETE && b.itemCount > 0);
  const recentAlbums = [...albums].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 4);
  if (drafts.length === 0 && !latest && recentAlbums.length === 0) return null;
  return (
    <section className="home-route__section" aria-label="Continue">
      <h2>Continue</h2>
      <div className="home-route__cards">
        {drafts.length > 0 && <InDevelop ids={drafts} count={summary?.draftCount ?? drafts.length} label={summary?.stripLabel ?? null} />}
        {latest && <LatestImport batch={latest} />}
        {recentAlbums.length > 0 && <RecentAlbums albums={recentAlbums} />}
      </div>
    </section>
  );
}
