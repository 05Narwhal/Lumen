import { useEffect, useRef } from "react";
import { useLocation } from "react-router";
import { filmStore } from "@/lib/film";
import { invoke } from "@/lib/ipc";
import { screenFor, slideshowPlayingStore } from "@/lib/presence";

/** Input tells Rust the user is still here; Rust clears the status after 15 minutes without it. */
const HEARTBEAT_MS = 60_000;

/**
 * Reports the coarse screen for the Discord status (Round 17): once when it changes, and as a
 * heartbeat on input. Only the fixed `PresenceScreen` word is sent, so no id or name can reach
 * Discord. Off, nothing is sent at all.
 */
export function usePresence(enabled: boolean) {
  const { pathname, search } = useLocation();
  const film = filmStore.use() !== null;
  const slideshowPlaying = slideshowPlayingStore.use();
  const screen = screenFor({ pathname, search, film, slideshowPlaying });
  const sentAt = useRef(0);

  useEffect(() => {
    if (!enabled) return;
    const send = () => {
      sentAt.current = Date.now();
      invoke("discord_set_screen", { screen }).catch(() => {});
    };
    send();
    const beat = () => {
      if (Date.now() - sentAt.current >= HEARTBEAT_MS) send();
    };
    window.addEventListener("pointerdown", beat, true);
    window.addEventListener("keydown", beat, true);
    return () => {
      window.removeEventListener("pointerdown", beat, true);
      window.removeEventListener("keydown", beat, true);
    };
  }, [enabled, screen]);
}
