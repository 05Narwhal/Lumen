import { Button } from "@mantine/core";
import { IconCircleCheck, IconMovie, IconRefresh } from "@tabler/icons-react";
import { useFfmpegStatus } from "@/hooks/use-media-status";
import "./styles.scss";

const platform = (): "windows" | "macos" | "linux" => {
  const ua = navigator.userAgent;
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac OS|Macintosh/i.test(ua)) return "macos";
  return "linux";
};

const INSTALL: Record<ReturnType<typeof platform>, { cmd: string; hint: string }> = {
  windows: { cmd: "winget install Gyan.FFmpeg", hint: "In a terminal, then restart Lumen or press Check again." },
  macos: { cmd: "brew install ffmpeg", hint: "With Homebrew, then press Check again." },
  linux: { cmd: "sudo apt install ffmpeg", hint: "Or your distribution's package, then press Check again." },
};

/**
 * One actionable card for the FFmpeg state (UX spec: missing codecs are a state, not an
 * error). `compact` is the Import-page banner; the full card lives in Settings.
 */
export function CodecCard({ compact = false }: { compact?: boolean }) {
  const { status, isFetching, check } = useFfmpegStatus();
  if (!status) return null;
  if (status.available) {
    if (compact) return null;
    return (
      <div className="codec-card" data-available="true" role="status">
        <IconCircleCheck size={20} className="codec-card__icon" />
        <div className="codec-card__body">
          <strong>Video and HEIC are ready</strong>
          <span className="codec-card__detail">{status.version}</span>
          <code className="codec-card__path">{status.path}</code>
        </div>
        <Button size="xs" variant="subtle" leftSection={<IconRefresh size={14} />} loading={isFetching} onClick={() => void check()}>
          Check again
        </Button>
      </div>
    );
  }
  const p = INSTALL[platform()];
  return (
    <div className="codec-card" data-available="false" data-compact={compact} role="status">
      <IconMovie size={20} className="codec-card__icon" />
      <div className="codec-card__body">
        <strong>Video and HEIC need FFmpeg</strong>
        <span className="codec-card__detail">
          They are catalogued now and get previews as soon as FFmpeg is installed. Install it with
        </span>
        <code className="codec-card__cmd">{p.cmd}</code>
        <span className="codec-card__detail">
          {p.hint} Or point <code>{status.envVar}</code> at a folder holding ffmpeg and ffprobe.
        </span>
      </div>
      <Button size="xs" variant="default" leftSection={<IconRefresh size={14} />} loading={isFetching} onClick={() => void check()}>
        Check again
      </Button>
    </div>
  );
}
