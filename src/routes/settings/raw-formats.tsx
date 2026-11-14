import { Button } from "@mantine/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { RawFormatsStatus } from "@/types/generated/RawFormatsStatus";
import { formatBytes } from "@/utils/format";
import { SettingGroup, SettingRow } from "./row";

/** Adobe's own page for the converter, Windows and macOS alike. Lumen never downloads it. */
export const CONVERTER_PAGE = "https://helpx.adobe.com/camera-raw/using/adobe-dng-converter.html";

export const rawFormatsQuery = {
  queryKey: queryKeys.rawFormats,
  queryFn: () => invoke<RawFormatsStatus>("get_raw_formats_status"),
};

/** The line Settings' list shows under *RAW formats*. */
export function rawFormatsLine(s: RawFormatsStatus | undefined): string | null {
  if (!s) return null;
  if (s.converter === "found") return "Converter found";
  if (s.converter === "not_found") return "Converter not found";
  return "Camera JPEG";
}

/**
 * Settings → RAW formats (Round 17): Adobe DNG Converter, which Lumen runs (only the copy the
 * person installed) on a RAW its own reader refuses, and the DNGs it keeps from that.
 */
export function RawFormatsSection() {
  const qc = useQueryClient();
  const status = useQuery(rawFormatsQuery);
  const [clearing, setClearing] = useState(false);
  const s = status.data;
  if (!s) return null;

  const clear = async () => {
    setClearing(true);
    try {
      const freed = await invoke<number>("clear_dng_cache");
      toast({ message: `${formatBytes(freed)} freed` });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not clear the converted copies." });
    } finally {
      setClearing(false);
      void qc.invalidateQueries({ queryKey: queryKeys.rawFormats });
    }
  };

  const linux = s.converter === "no_linux_version";
  return (
    <>
      <SettingGroup title="Adobe DNG Converter">
        {s.converter === "found" && (
          <SettingRow
            label="Found"
            hint="RAW files Lumen can't read itself are converted with it when you open them in Develop. Lumen runs the copy you installed and never changes your originals."
          >
            <code className="settings__code">{s.converterPath}</code>
          </SettingRow>
        )}
        {s.converter === "not_found" && (
          <SettingRow
            label="Not installed"
            hint="Without it, RAW files Lumen can't read itself open as the camera's JPEG, marked Camera rendering, not RAW. Adobe offers it free; once it's installed, Lumen finds it."
          >
            <Button size="xs" onClick={() => void openUrl(CONVERTER_PAGE)}>
              Get Adobe DNG Converter
            </Button>
            <Button size="xs" variant="default" loading={status.isFetching} onClick={() => void status.refetch()}>
              Check again
            </Button>
          </SettingRow>
        )}
        {linux && <SettingRow label="Not available" hint="Adobe makes no Linux version. RAW files Lumen can't read itself open as the camera's JPEG, marked Camera rendering, not RAW." />}
      </SettingGroup>
      {(!linux || s.cacheBytes > 0) && (
        <SettingGroup>
          <SettingRow label="Converted copies" hint="DNGs kept so a converted photo opens at once. Up to 20 GB, the longest unopened go first.">
            <span className="settings__value">{formatBytes(s.cacheBytes)}</span>
            <Button size="xs" variant="default" disabled={s.cacheBytes === 0} loading={clearing} onClick={() => void clear()}>
              Clear
            </Button>
          </SettingRow>
        </SettingGroup>
      )}
    </>
  );
}
