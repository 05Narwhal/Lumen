import { Button, Modal } from "@mantine/core";
import { useEffect, useRef, useState } from "react";
import { toast } from "@/components/toast";
import { invoke } from "@/lib/ipc";
import { ThemeMiniature } from "@/routes/plugins/theme-miniature";
import type { ThemeInspect } from "@/types/generated/ThemeInspect";
import { closeThemeInstall, themeInstallStore } from "./store";
import "./styles.scss";

/**
 * The sheet for a `.lumentheme` opened from a file: what it is called, who made it, its licence
 * and a miniature of it. A theme runs no code and asks for nothing, so there is no permission
 * list; *Install* is `theme_install { path }`.
 */
export function ThemeInstallSheet({ p, busy, error, onInstall }: { p: ThemeInspect; busy: boolean; error: string | null; onInstall: () => void }) {
  const t = p.theme;
  const scheme = t.defaultScheme === "light" ? "light" : "dark";
  return (
    <>
      <div className="plugin-install__head">
        <div className="plugin-install__title theme-install__title">
          <span>{t.name}</span>
          <span className="plugin-install__version">{t.version}</span>
        </div>
        {t.layout && <span className="theme-install__badge">Layout</span>}
      </div>
      <div className="plugin-install__trust">{t.author}</div>
      <div className="theme-install__miniature">
        <ThemeMiniature tokens={t.tokens} scheme={scheme} />
      </div>
      {t.description && <p className="plugin-install__text">{t.description}</p>}
      <p className="plugin-install__text">It changes how Lumen looks{t.layout ? " and where things sit" : ""}. It runs no code and can't reach your photos.</p>
      <div className="plugin-install__licence">
        <span className="plugin-install__label">Licence</span>
        <span>{t.license}</span>
      </div>
      {p.problem && <p className="plugin-install__problem">{p.problem}</p>}
      {error && <p className="plugin-install__problem" role="alert">{error}</p>}
      <div className="plugin-install__actions">
        <Button variant="default" onClick={closeThemeInstall} disabled={busy} data-autofocus>
          {p.problem ? "Close" : "Cancel"}
        </Button>
        {!p.problem && (
          <Button onClick={onInstall} disabled={busy}>
            Install
          </Button>
        )}
      </div>
    </>
  );
}

/** Mounted with the plugin install host: shows the sheet for the theme file `themeInstallStore` holds. */
export function ThemeInstallHost() {
  const { path } = themeInstallStore.use();
  const [data, setData] = useState<{ path: string; inspect: ThemeInspect } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const installing = useRef(false);

  useEffect(() => {
    installing.current = false;
    setBusy(false);
    setError(null);
    setData(null);
    if (!path) return;
    let live = true;
    invoke<ThemeInspect>("theme_inspect", { path }).then(
      (inspect) => live && setData({ path, inspect }),
      (e: { message?: string }) => {
        if (!live) return;
        toast({ message: e?.message ?? "Could not read this theme." });
        closeThemeInstall();
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
      const t = await invoke<{ name: string }>("theme_install", { path });
      toast({ message: `Installed ${t.name}` });
      closeThemeInstall();
    } catch (e) {
      installing.current = false;
      setBusy(false);
      setError((e as { message?: string })?.message ?? "Could not install this theme.");
    }
  };

  const shown = path && data?.path === path ? data.inspect : null;
  return (
    <Modal
      opened={shown !== null}
      onClose={() => !busy && closeThemeInstall()}
      data-part="install-sheet"
      closeOnEscape={!busy}
      closeOnClickOutside={!busy}
      withCloseButton={false}
      centered
      size="md"
      classNames={{ content: "plugin-install", header: "plugin-install__header", body: "plugin-install__body" }}
    >
      {shown && <ThemeInstallSheet p={shown} busy={busy} error={error} onInstall={install} />}
    </Modal>
  );
}
