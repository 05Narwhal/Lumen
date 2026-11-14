import { IconDeviceMobile } from "@tabler/icons-react";
import "./styles.scss";

const platform = (): "windows" | "macos" | "linux" => {
  const ua = navigator.userAgent;
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac OS|Macintosh/i.test(ua)) return "macos";
  return "linux";
};

const INSTALL: Record<ReturnType<typeof platform>, { what: string; hint: string }> = {
  windows: { what: "the Apple Devices app or iTunes", hint: "Install it from the Microsoft Store, then plug the phone in again." },
  macos: { what: "macOS's built-in device support", hint: "It should already be there. Reconnect the phone or restart the Mac." },
  linux: { what: "usbmuxd", hint: "Install the usbmuxd package for your distribution, then plug the phone in again." },
};

/**
 * Shown in place of the iPhone list when usbmuxd cannot be reached. Missing drivers are a
 * state with one action, never an error (UX spec §9).
 */
export function UsbmuxdCard() {
  const p = INSTALL[platform()];
  return (
    <div className="codec-card" data-available="false" role="status">
      <IconDeviceMobile size={20} className="codec-card__icon" />
      <div className="codec-card__body">
        <strong>iPhone import needs {p.what}</strong>
        <span className="codec-card__detail">
          Lumen talks to iPhones through Apple's device service. {p.hint}
        </span>
      </div>
    </div>
  );
}
