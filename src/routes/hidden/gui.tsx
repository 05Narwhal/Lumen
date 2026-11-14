import { Button } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { IconArrowBackUp, IconLock } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo } from "react";
import { useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { PhotoGrid } from "@/components/photo-grid";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { useSettings } from "@/hooks/use-settings";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { moveToLibrary } from "@/lib/vault";
import { ROW_HEIGHTS } from "@/routes/library";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { VaultInfo } from "@/types/generated/VaultInfo";
import { VaultLockScreen } from "./lock";
import "./styles.scss";

/**
 * `/hidden`: photos moved here appear nowhere else. Rust holds the lock; this page asks for
 * the password, shows the folder while it is open, and locks it again on the way out.
 */
export function HiddenRoute() {
  const qc = useQueryClient();
  const { settings } = useSettings();
  const [, setSearchParams] = useSearchParams();
  const status = useQuery({ queryKey: queryKeys.vault, queryFn: () => invoke<VaultInfo>("vault_status") });
  const scope = useMemo<LayoutScope>(() => ({ ...LIBRARY_SCOPE, vault: true }), []);
  const onOpen = useCallback(
    (assetId: number) =>
      setSearchParams((p) => {
        p.set("asset", String(assetId));
        return p;
      }),
    [setSearchParams],
  );
  const lock = useCallback(() => {
    void invoke("vault_lock").then(() => qc.invalidateQueries({ queryKey: queryKeys.vault }));
  }, [qc]);
  // Leaving the folder locks it.
  useEffect(() => () => void invoke("vault_lock").catch(() => {}), []);

  const s = status.data;
  if (!s) return <div className="hidden-route" />;
  if (!s.unlocked) {
    return (
      <div className="hidden-route hidden-route--locked">
        <VaultLockScreen hasPassword={s.hasPassword} onOpened={() => void qc.invalidateQueries({ queryKey: queryKeys.vault })} />
      </div>
    );
  }
  return (
    <div className="hidden-route">
      <PageHeader
        title="Hidden"
        search={false}
        toolbar={
          <Button size="xs" variant="default" leftSection={<IconLock size={14} />} onClick={lock}>
            Lock
          </Button>
        }
      />
      {s.count === 0 ? (
        <EmptyState
          icon={<IconLock />}
          title="Nothing hidden yet"
          description="Right-click photos anywhere in Lumen and choose Move to Hidden. They will appear only here."
        />
      ) : (
        <div className="hidden-route__grid">
          <PhotoGrid
            rowHeight={ROW_HEIGHTS[settings?.appearance.tileSize ?? "m"]}
            scope={scope}
            onOpen={onOpen}
            extraMenuItems={(ids) => [
              { label: "Move to Library", icon: <IconArrowBackUp size={16} />, onSelect: () => void moveToLibrary(qc, ids) },
            ]}
          />
        </div>
      )}
    </div>
  );
}
