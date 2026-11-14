import { Tooltip } from "@mantine/core";
import { IconPlayerPause, IconPlayerPlay, IconVolume, IconVolumeOff } from "@tabler/icons-react";
import type { ReactNode } from "react";
import { formatDuration } from "@/utils/date";
import "./styles.scss";

/**
 * The bar under a playing video, shared by the viewer's videos and a memory's film (Round 6):
 * play / pause, the time, a seek bar, the length, mute, then whatever the player adds (loop
 * for videos, full screen for films). Times are in milliseconds.
 */
export function MediaControls({
  playing,
  onToggle,
  timeMs,
  durationMs,
  onSeek,
  muted,
  onToggleMute,
  disabled = false,
  hidden = false,
  children,
}: {
  playing: boolean;
  onToggle: () => void;
  timeMs: number;
  durationMs: number;
  onSeek: (ms: number) => void;
  muted: boolean;
  onToggleMute: () => void;
  disabled?: boolean;
  /** Idle (Round 8B): the bar fades out with the rest of the viewer's controls. */
  hidden?: boolean;
  /** More buttons after mute: `ControlButton`s. */
  children?: ReactNode;
}) {
  return (
    // While the pointer rests on the bar the viewer never goes idle (`useIdle`).
    <div className="media-controls" data-hidden={hidden} data-keep-awake>
      <ControlButton label={playing ? "Pause" : "Play"} tip={playing ? "Pause (Space)" : "Play (Space)"} onClick={onToggle} disabled={disabled}>
        {playing ? <IconPlayerPause size={18} /> : <IconPlayerPlay size={18} />}
      </ControlButton>
      <span className="media-controls__time">{formatDuration(timeMs)}</span>
      <input
        type="range"
        className="media-controls__scrubber"
        aria-label="Seek"
        min={0}
        max={durationMs || 0}
        step={50}
        value={Math.min(timeMs, durationMs || 0)}
        disabled={disabled}
        onChange={(e) => onSeek(Number(e.currentTarget.value))}
      />
      <span className="media-controls__time">{formatDuration(durationMs)}</span>
      <ControlButton label={muted ? "Unmute" : "Mute"} pressed={muted} onClick={onToggleMute} disabled={disabled}>
        {muted ? <IconVolumeOff size={18} /> : <IconVolume size={18} />}
      </ControlButton>
      {children}
    </div>
  );
}

/** One round button in the bar, with its tooltip. */
export function ControlButton({
  label,
  tip,
  pressed,
  onClick,
  disabled,
  children,
}: {
  label: string;
  tip?: string;
  pressed?: boolean;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <Tooltip label={tip ?? label} openDelay={600} withArrow>
      <button type="button" className="media-controls__icon" aria-label={label} aria-pressed={pressed} onClick={onClick} disabled={disabled}>
        {children}
      </button>
    </Tooltip>
  );
}
