import { Button, Modal } from "@mantine/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { toast } from "@/components/toast";
import { inspectPluginPackage, installPlugin, takePendingPluginPackage } from "@/lib/plugins";
import type { PluginInspect } from "@/types/generated/PluginInspect";
import type { PluginPermission } from "@/types/generated/PluginPermission";
import type { PluginTrust } from "@/types/generated/PluginTrust";
import { closePluginInstall, openPluginInstall, pluginInstallStore } from "./store";
import { ThemeInstallHost } from "./theme-sheet";
import "./styles.scss";

/** Spec §3's wording of each permission. */
export const PERMISSION_LABELS: Record<PluginPermission, string> = {
  importReadChosen: "Read the folder or file you choose when you import with it",
  photosRead: "See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place",
  developWriteDrafts: "Change Develop drafts of those photos (every change can be undone)",
  storage: "Keep its own files (up to 50 MB)",
  // API 1.1 (Round 24 §8.4–§8.7)
  models: "Use AI models it downloads",
  libraryRead: "Read dates, places and file names of your photos",
  placesWrite: "Suggest places for photos",
  originalsGps: "Add GPS to original files that have none",
  // API 1.3 (Round 25 P4b): proposals, shown for review; nothing changes until accepted.
  datesWrite: "Suggest dates for photos",
  tagsWrite: "Suggest tags and labels for photos",
  captionsWrite: "Suggest titles and captions for photos",
  peopleWrite: "Suggest names for faces Lumen found",
};

/** The *It can never* line; `originals.gps` and providers narrow it (Round 24 §8.1, §8.7). */
export function neverText(permissions: PluginPermission[], adds: string[]): string {
  const originals = permissions.includes("originalsGps")
    ? "Change anything else in your originals or your library"
    : "Touch your originals or your library";
  const programs = adds.some((a) => a.startsWith("Open ")) ? "start programs other than the ones above" : "start other programs";
  return `${originals}, use the internet, or ${programs}.`;
}

export function trustLine(t: PluginTrust): string {
  if (t.type === "jlStudios") return "✓ From JL Studios";
  if (t.type === "author") return `Signed by ${t.name} · key ${t.fingerprint}`;
  return "⚠ Unsigned";
}

const OPEN_PACKAGE = "plugins://open-package";

/** Mount once at the app root: listens for packages being opened and shows the install sheet. */
export function PluginInstallHost() {
  const { path } = pluginInstallStore.use();
  const [data, setData] = useState<{ path: string; inspect: PluginInspect } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const installing = useRef(false);

  useEffect(() => {
    const un = listen<string>(OPEN_PACKAGE, (e) => openPluginInstall(e.payload));
    void takePendingPluginPackage()
      .then((p) => {
        if (p) openPluginInstall(p);
      })
      .catch(() => undefined);
    return () => void un.then((f) => f());
  }, []);

  useEffect(() => {
    // A new package starts a new install: the host stays mounted, so the last success must not
    // leave `installing` set (it once blocked every install after the first).
    installing.current = false;
    setBusy(false);
    setError(null);
    setData(null);
    if (!path) return;
    let live = true;
    inspectPluginPackage(path).then(
      (inspect) => {
        if (live) setData({ path, inspect });
      },
      () => {
        if (!live) return;
        toast({ message: "Could not read this plugin." });
        closePluginInstall();
      },
    );
    return () => {
      live = false;
    };
  }, [path]);

  const install = async () => {
    if (!path || installing.current) return;
    installing.current = true;
    setBusy(true);
    try {
      const info = await installPlugin(path);
      toast({ message: `Installed ${info.name}` });
      closePluginInstall();
    } catch (e) {
      installing.current = false;
      setBusy(false);
      setError((e as { message?: string })?.message ?? "Could not install this plugin.");
    }
  };

  const shown = path && data?.path === path ? data.inspect : null;
  return (
    <>
    <ThemeInstallHost />
    <Modal
      opened={shown !== null}
      onClose={() => !busy && closePluginInstall()}
      data-part="install-sheet"
      closeOnEscape={!busy}
      closeOnClickOutside={!busy}
      title={shown?.update ? `Update ${shown.update.from} → ${shown.info.version}` : undefined}
      withCloseButton={false}
      centered
      size="md"
      classNames={{ content: "plugin-install", header: "plugin-install__header", body: "plugin-install__body" }}
    >
      {shown && <PluginInstallSheet p={shown} busy={busy} error={error} onInstall={install} />}
    </Modal>
    </>
  );
}

/** The sheet's content, with no host around it (also shown by /dev/ui). */
export function PluginInstallSheet({ p, busy, error, onInstall }: { p: PluginInspect; busy: boolean; error: string | null; onInstall: () => void }) {
  const { info } = p;
  const newPerms = new Set(p.update?.newPermissions ?? []);
  const blocked = p.problems.length > 0 || p.older !== null;
  const codeless = !info.kinds.some((k) => k === "importer" || k === "tools");
  return (
    <>
      <div className="plugin-install__head">
        {info.iconUrl ? <img className="plugin-install__icon" src={info.iconUrl} alt="" /> : <div className="plugin-install__icon" aria-hidden />}
        <div>
          <div className="plugin-install__title">
            <span>{info.name}</span>
            <span className="plugin-install__version">{info.version}</span>
          </div>
          <div className="plugin-install__trust">
            {info.author} · {trustLine(info.trust)}
          </div>
        </div>
      </div>
      {p.adds.length > 0 && (
        <div className="plugin-install__section">
          <div className="plugin-install__label">Adds</div>
          {p.adds.map((a) => (
            <p key={a} className="plugin-install__text">{a}</p>
          ))}
        </div>
      )}
      <div className="plugin-install__section">
        <div className="plugin-install__label">It can</div>
        {codeless ? (
          <p className="plugin-install__text">It runs no code.</p>
        ) : (
          <ul className="plugin-install__list">
            {info.permissions.map((perm) => (
              <li key={perm}>
                {PERMISSION_LABELS[perm]}
                {newPerms.has(perm) && <span className="plugin-install__new">New</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="plugin-install__section">
        <div className="plugin-install__label">It can never</div>
        <p className="plugin-install__text">{neverText(info.permissions, p.adds)}</p>
      </div>
      {p.notes.length > 0 && (
        <ul className="plugin-install__list">
          {p.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      <div className="plugin-install__licence">
        <span className="plugin-install__label">Licence</span>
        <span>{info.license}</span>
      </div>
      {p.older !== null && <p className="plugin-install__problem">Older than the installed {p.older}</p>}
      {p.problems.map((x) => (
        <p key={x} className="plugin-install__problem">{x}</p>
      ))}
      {error && <p className="plugin-install__problem" role="alert">{error}</p>}
      <div className="plugin-install__actions">
        <Button variant="default" onClick={closePluginInstall} disabled={busy} data-autofocus>
          {blocked ? "Close" : "Cancel"}
        </Button>
        {!blocked && (
          <Button onClick={onInstall} disabled={busy}>
            Install
          </Button>
        )}
      </div>
    </>
  );
}
