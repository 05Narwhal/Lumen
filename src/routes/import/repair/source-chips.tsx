import { IconBrandApple, IconDeviceMobile, IconDeviceSdCard, IconDeviceUsb, IconFolder } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { pickFolder } from "@/lib/dialogs";
import type { ReactNode } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { RepairSource } from "@/types/generated/RepairSource";
import type { RepairSourceRef } from "@/types/generated/RepairSourceRef";
import type { SourceFolder } from "@/types/generated/SourceFolder";

export const baseName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;

const icon = (s: RepairSource): ReactNode => {
  if (s.ref.kind === "iphone") return <IconDeviceMobile size={16} />;
  if (s.ref.kind === "macDisk") return <IconBrandApple size={16} />;
  return s.removable ? <IconDeviceSdCard size={16} /> : <IconDeviceUsb size={16} />;
};

/**
 * *Check against* (Round 22 B4): one chip per place the originals may be, and *A folder…*. A source
 * that is not ready is off with its reason as the hint. *Restore…* (`?assets=`) puts its
 * *Look in <path>* offers first.
 */
export function SourceChips({
  onCheck,
  restoring,
  disabled,
}: {
  onCheck: (ref: RepairSourceRef, name: string) => void;
  restoring: { ids: number[]; folders: SourceFolder[] } | null;
  disabled: boolean;
}) {
  const sources = useQuery({ queryKey: queryKeys.repairSources, queryFn: () => invoke<RepairSource[]>("list_repair_sources"), staleTime: 5_000 }).data ?? [];
  const folder = async () => {
    const picked = await pickFolder({ title: "Choose the folder to check" });
    if (typeof picked === "string") onCheck({ kind: "folder", id: picked }, baseName(picked));
  };
  return (
    <section className="repair-sources" aria-label="Check against">
      <h2>{restoring ? (restoring.ids.length === 1 ? "Where is a good copy of this file?" : `Where is a good copy of these ${restoring.ids.length.toLocaleString()} files?`) : "Check against"}</h2>
      <div className="repair-sources__chips">
        {restoring?.folders.map((f) => (
          <button key={f.path} type="button" className="repair-chip" disabled={disabled} title="Where they were imported from" onClick={() => onCheck({ kind: "folder", id: f.path }, baseName(f.path))}>
            <IconFolder size={16} /> {`Look in ${f.path}`}
          </button>
        ))}
        {sources.map((s) => (
          <button
            key={s.ref.id}
            type="button"
            className="repair-chip"
            disabled={disabled || !s.ready}
            title={s.ready ? s.detail : s.detail}
            data-source={s.ref.id}
            onClick={() => onCheck(s.ref, s.name)}
          >
            {icon(s)} {s.name}
            {!s.ready && <small>{s.detail}</small>}
          </button>
        ))}
        <button type="button" className="repair-chip" disabled={disabled} onClick={() => void folder()}>
          <IconFolder size={16} /> A folder…
        </button>
      </div>
    </section>
  );
}
