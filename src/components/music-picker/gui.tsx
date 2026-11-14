import { ActionIcon, Button, Modal, Tabs, TextInput } from "@mantine/core";
import { IconFolder, IconPlayerPlayFilled, IconPlayerStopFilled, IconSearch, IconTrash, IconVolume } from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import { Segmented } from "@/components/segmented";
import { Spinner } from "@/components/spinner";
import { useLibraryMusic, useMusicEnabled, useMusicSearch, useSaveMusic } from "@/hooks/use-music";
import type { MusicQuery } from "@/types/generated/MusicQuery";
import type { MusicTrack } from "@/types/generated/MusicTrack";
import type { SavedTrack } from "@/types/generated/SavedTrack";
import { formatDuration } from "@/utils/date";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

const VIBES = ["happy", "calm", "epic", "energetic", "sad", "romantic", "dreamy"];
const GENRES = ["pop", "electronic", "ambient", "rock", "jazz", "classical", "hiphop", "folk"];
const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

const VOLUME_KEY = "lumen.music-preview-volume";
const DEFAULT_VOLUME = 0.6;

/**
 * The slider read as the show's level. Full is the track as recorded; the bottom of the
 * slider is silence rather than a number FFmpeg would refuse.
 */
export function gainForVolume(volume: number): number {
  if (volume <= 0) return -30;
  return Math.max(-30, Math.min(10, 20 * Math.log10(volume)));
}

/** A private window throws on both of these, and a picker that cannot remember is fine. */
function rememberedVolume(): number {
  try {
    const stored = Number(localStorage.getItem(VOLUME_KEY));
    return Number.isFinite(stored) && stored > 0 ? Math.min(1, stored) : DEFAULT_VOLUME;
  } catch {
    return DEFAULT_VOLUME;
  }
}

/** One row, whether it is already in the library or still out on Jamendo. */
function Row({
  id,
  title,
  artist,
  durationMs,
  licence,
  playable,
  playing,
  onPlay,
  actions,
}: {
  id: string;
  title: string;
  artist: string;
  durationMs: number;
  licence: string;
  playable: boolean;
  playing: boolean;
  onPlay: (id: string | null) => void;
  actions: React.ReactNode;
}) {
  return (
    <div className="music-picker__row">
      {playable && (
        <ActionIcon
          variant="subtle"
          color="gray"
          aria-label={playing ? `Stop ${title}` : `Play ${title}`}
          onClick={() => onPlay(playing ? null : id)}
        >
          {playing ? <IconPlayerStopFilled size={16} /> : <IconPlayerPlayFilled size={16} />}
        </ActionIcon>
      )}
      <span className="music-picker__text">
        <span className="music-picker__title">{title}</span>
        <span className="music-picker__meta">
          {[artist, durationMs ? formatDuration(durationMs) : null, licence].filter(Boolean).join(" · ")}
        </span>
      </span>
      {actions}
    </div>
  );
}

/**
 * Music for a slideshow: what the library already holds, what Jamendo offers, or a file
 * from this computer. Only tracks whose licence allows a derivative work are listed, so
 * anything here may legally sit under an exported film.
 *
 * One audio element serves every row, because two tracks at once is never what anyone
 * wanted, and the volume the user listens at becomes the show's level.
 */
export function MusicPicker({
  onClose,
  onUse,
  onChooseFile,
  initialTab,
}: {
  onClose: () => void;
  onUse: (track: SavedTrack, gainDb: number) => void;
  onChooseFile: () => void;
  /** Which tab to open on; only /dev/ui passes it, so a screenshot can show either. */
  initialTab?: string;
}) {
  const { tracks: library, isLoading } = useLibraryMusic();
  const { save, remove, saving } = useSaveMusic();
  const enabled = useMusicEnabled();
  const [text, setText] = useState("");
  const [vibe, setVibe] = useState("");
  const [genre, setGenre] = useState("");
  const [tab, setTab] = useState<string | null>(initialTab ?? "library");
  const [volume, setVolume] = useState(rememberedVolume);
  /** The id of the row that is playing, which is a saved track's path or a Jamendo id. */
  const [playing, setPlaying] = useState<string | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement>(null);

  useEffect(() => {
    if (audio.current) audio.current.volume = volume;
  }, [volume, source]);

  useEffect(() => {
    const el = audio.current;
    if (!el) return;
    if (!source) {
      el.pause();
      return;
    }
    el.volume = volume;
    // jsdom returns undefined here, and a real browser returns a promise that rejects when
    // the file will not play; neither may throw out of an effect.
    try {
      void Promise.resolve(el.play()).catch(() => setPlaying(null));
    } catch {
      setPlaying(null);
    }
    // `volume` is applied by the effect above; re-playing on a volume change would restart.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source]);

  const play = (id: string | null, src: string | null) => {
    setPlaying(id);
    setSource(id ? src : null);
  };

  const query = useMemo<MusicQuery>(
    () => ({
      text: text.trim() || null,
      vibe: vibe || null,
      genre: genre || null,
      instrumentalOnly: false,
      tempo: null,
      includeNoDerivatives: false,
      limit: 40,
    }),
    [text, vibe, genre],
  );
  const search = useMusicSearch(query, enabled && tab === "find");

  const use = (track: SavedTrack) => {
    play(null, null);
    onUse(track, gainForVolume(volume));
  };
  const useFound = async (track: MusicTrack) => {
    const savedTrack = await save(track).catch(() => null);
    if (savedTrack) use(savedTrack);
  };

  return (
    <Modal opened onClose={onClose} title="Music" size="lg" centered classNames={{ body: "music-picker" }}>
      <Tabs value={tab} onChange={setTab}>
        <Tabs.List>
          <Tabs.Tab value="library">In your library</Tabs.Tab>
          <Tabs.Tab value="find">Find music</Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="library" className="music-picker__panel">
          {isLoading ? (
            <Spinner size="sm" label="Loading music" />
          ) : library.length === 0 ? (
            <p className="music-picker__empty">No music yet. Find some on the next tab, or use a file from this computer.</p>
          ) : (
            library.map((t) => (
              <Row
                key={t.path}
                id={t.path}
                title={t.title}
                artist={t.artist}
                durationMs={t.durationMs}
                licence={t.licenceName}
                playable
                playing={playing === t.path}
                onPlay={(id) => play(id, mediaUrl("library-music", t.path.split(/[\\/]/).pop() ?? ""))}
                actions={
                  <>
                    <Button size="xs" aria-label={`Use ${t.title}`} onClick={() => use(t)}>
                      Use
                    </Button>
                    <ActionIcon variant="subtle" color="red" aria-label={`Remove ${t.title}`} onClick={() => void remove(t.path)}>
                      <IconTrash size={16} />
                    </ActionIcon>
                  </>
                }
              />
            ))
          )}
        </Tabs.Panel>

        <Tabs.Panel value="find" className="music-picker__panel">
          {!enabled ? (
            <p className="music-picker__empty">
              Lumen can browse Jamendo’s Creative Commons catalogue. It needs a free client id first, and asks for nothing until
              you add one. <Link to="/settings">Open Settings → Music</Link>.
            </p>
          ) : (
            <>
              <TextInput
                role="searchbox"
                aria-label="Search for music"
                placeholder="Piano, sunrise, drums…"
                leftSection={<IconSearch size={16} />}
                value={text}
                onChange={(e) => setText(e.currentTarget.value)}
              />
              <Segmented
                label="Vibe"
                size="xs"
                value={vibe}
                onChange={setVibe}
                data={[{ label: "Any", value: "" }, ...VIBES.map((v) => ({ label: capital(v), value: v }))]}
              />
              <Segmented
                label="Genre"
                size="xs"
                value={genre}
                onChange={setGenre}
                data={[{ label: "Any", value: "" }, ...GENRES.map((g) => ({ label: capital(g), value: g }))]}
              />
              {search.isLoading ? (
                <Spinner size="sm" label="Searching" />
              ) : search.error ? (
                <p className="music-picker__empty">Could not reach Jamendo just now.</p>
              ) : (search.data ?? []).length === 0 ? (
                <p className="music-picker__empty">Nothing found. Try another word or fewer filters.</p>
              ) : (
                (search.data ?? []).map((t) => (
                  <Row
                    key={t.id}
                    id={t.id}
                    title={t.title}
                    artist={t.artist}
                    durationMs={t.durationMs}
                    licence={t.licenceName}
                    playable
                    playing={playing === t.id}
                    onPlay={(id) => play(id, mediaUrl("music-preview", t.id))}
                    actions={
                      <Button size="xs" aria-label={`Use ${t.title}`} loading={saving} onClick={() => void useFound(t)}>
                        Use
                      </Button>
                    }
                  />
                ))
              )}
            </>
          )}
        </Tabs.Panel>
      </Tabs>

      <div className="music-picker__actions">
        <label className="music-picker__volume" title={`Preview volume ${Math.round(volume * 100)}%`}>
          <IconVolume size={16} />
          <input
            type="range"
            aria-label="Preview volume"
            min={0}
            max={100}
            value={Math.round(volume * 100)}
            onChange={(e) => {
              const v = Number(e.currentTarget.value) / 100;
              setVolume(v);
              try {
                localStorage.setItem(VOLUME_KEY, String(v));
              } catch {
                // A private window refuses; the slider still works for this session.
              }
            }}
          />
        </label>
        <Button variant="default" leftSection={<IconFolder size={16} />} onClick={onChooseFile}>
          Use a file…
        </Button>
        <Button variant="subtle" onClick={onClose}>
          Close
        </Button>
      </div>
      {/* One element for every row: starting a second track stops the first. */}
      <audio ref={audio} src={source ?? undefined} preload="none" onEnded={() => play(null, null)} />
    </Modal>
  );
}
