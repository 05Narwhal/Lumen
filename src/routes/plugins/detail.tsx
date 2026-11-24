import { openUrl } from "@tauri-apps/plugin-opener";

export const openLink = (url: string) => void openUrl(url).catch(() => window.open(url, "_blank", "noopener"));

/** A store button: *Get* downloads, *Open* goes to the installed plugin's settings, *Update* updates. */
export type CardAction = { label: "Get" | "Open" | "Update"; disabled: boolean };
