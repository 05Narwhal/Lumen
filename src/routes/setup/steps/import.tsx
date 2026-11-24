import { Button } from "@mantine/core";
import { IconDeviceMobile, IconDeviceSdCard, IconFolderOpen } from "@tabler/icons-react";
import { pickFolder as pickImportFolder } from "@/lib/dialogs";
import { useState } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { useVolumes } from "@/hooks/use-devices";
import { useIphones } from "@/hooks/use-iphones";
import { useImport } from "@/hooks/use-import";
import { isAppError } from "@/lib/ipc";

export function ImportStep({ onBack }: { onBack: () => void }) {
  const navigate = useNavigate();
  const { startFolderImport } = useImport();
  const { volumes, startImport: startVolumeImport } = useVolumes();
  const { devices: iphones, startImport: startIphoneImport } = useIphones();
  const [busy, setBusy] = useState(false);

  const importPhone = async (udid: string) => {
    setBusy(true);
    try {
      await startIphoneImport(udid);
      navigate("/", { replace: true });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };

  const importCard = async (mount: string) => {
    setBusy(true);
    try {
      await startVolumeImport(mount);
      navigate("/", { replace: true });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };

  const pickFolder = async () => {
    const dir = await pickImportFolder({ title: "Choose a folder to import" });
    if (typeof dir !== "string") return;
    setBusy(true);
    try {
      await startFolderImport({ path: dir });
      navigate("/", { replace: true });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="setup__step">
      <h1 className="setup__title">Add your first photos</h1>
      <p className="setup__lead">Files are copied into your library; nothing on the source is changed.</p>
      <div className="setup__tiles">
        <button type="button" className="setup__tile" onClick={pickFolder} disabled={busy}>
          <IconFolderOpen size={28} />
          <strong>Choose a folder</strong>
          <span>Photos and videos inside, subfolders included</span>
        </button>
        {volumes.map((v) => (
          <button key={v.mount} type="button" className="setup__tile" onClick={() => void importCard(v.mount)} disabled={busy || v.newItems === 0}>
            <IconDeviceSdCard size={28} />
            <strong>{v.label}</strong>
            <span>{v.newItems === 0 ? "Nothing new on this card" : `${v.newItems.toLocaleString()} new photos and videos`}</span>
          </button>
        ))}
        {iphones.map((p) => (
          <button key={p.udid} type="button" className="setup__tile" onClick={() => void importPhone(p.udid)} disabled={busy || !p.paired}>
            <IconDeviceMobile size={28} />
            <strong>{p.name}</strong>
            <span>{p.paired ? "Import everything in Camera Roll" : "Unlock it and tap Trust"}</span>
          </button>
        ))}
        {volumes.length === 0 && iphones.length === 0 && (
          <div className="setup__tile setup__tile--soon" aria-disabled>
            <IconDeviceMobile size={28} />
            <strong>Memory card or iPhone</strong>
            <span>Plug one in and it appears here</span>
          </div>
        )}
      </div>
      <div className="setup__actions">
        <Button variant="subtle" onClick={onBack}>
          Back
        </Button>
        <Button variant="default" onClick={() => navigate("/", { replace: true })}>
          Skip for now
        </Button>
      </div>
    </div>
  );
}
