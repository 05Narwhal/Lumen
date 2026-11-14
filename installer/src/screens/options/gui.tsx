import { Button, SwitchRow } from "@/components/controls";
import { Actions, Card, Heading, Small, Spacer } from "@/components/layout";
import { formatBytes, shortPath } from "@/lib/format";
import type { FolderCheck } from "@/types";
import "./styles.scss";

export interface OptionValues {
  desktopShortcut: boolean;
  startWithWindows: boolean;
  downloadModels: boolean;
}

interface Props extends OptionValues {
  folder: string;
  /** null while the folder is being checked. */
  check: FolderCheck | null;
  modelsBytes: number;
  /** What Lumen itself needs on disk. */
  needBytes: number;
  onChange: (patch: Partial<OptionValues>) => void;
  onChangeFolder: () => void;
  onBack: () => void;
  onInstall: () => void;
}

/** Why a folder is refused, in words that say what to do. */
export function folderProblem(check: FolderCheck | null, needBytes: number): string | null {
  switch (check?.kind) {
    case "needs_admin":
      return "This folder needs administrator rights. Pick one inside your user folder.";
    case "not_empty":
      return "This folder has other files in it. Pick an empty folder or a new one.";
    case "no_space":
      return `Not enough space on that drive. Free up ${formatBytes(needBytes)} or pick another drive.`;
    case "is_file":
      return "That path is a file. Pick a folder.";
    default:
      return null;
  }
}

export function Options({ folder, check, desktopShortcut, startWithWindows, downloadModels, modelsBytes, needBytes, onChange, onChangeFolder, onBack, onInstall }: Props) {
  const problem = folderProblem(check, needBytes + (downloadModels ? modelsBytes : 0));
  const total = needBytes + (downloadModels ? modelsBytes : 0);
  return (
    <>
      <Heading>Install options</Heading>
      <div>
        <div className="options__path">
          <code title={folder}>{shortPath(folder, 64)}</code>
          <Button onClick={onChangeFolder}>Change…</Button>
        </div>
        {problem && (
          <p role="alert" className="options__problem">
            {problem}
          </p>
        )}
      </div>
      <Card>
        <SwitchRow label="Desktop shortcut" detail="An icon on the desktop." checked={desktopShortcut} onChange={(v) => onChange({ desktopShortcut: v })} />
        <SwitchRow label="Start with Windows" detail="Lumen opens when you sign in." checked={startWithWindows} onChange={(v) => onChange({ startWithWindows: v })} />
        <SwitchRow
          label="Download smart features now"
          detail={
            downloadModels
              ? `${formatBytes(modelsBytes)}: search by description, faces, pets, music beats, denoise. Otherwise Lumen fetches them later.`
              : "Off: Lumen downloads them the first time it opens."
          }
          checked={downloadModels}
          onChange={(v) => onChange({ downloadModels: v })}
        />
      </Card>
      <Actions>
        <Small className="s-spacer">
          Needs {formatBytes(total)}
          {check ? ` · ${formatBytes(check.free)} free` : ""}
        </Small>
        <Spacer />
        <Button variant="link" onClick={onBack}>Back</Button>
        <Button variant="primary" disabled={check === null || problem !== null} onClick={onInstall}>Install</Button>
      </Actions>
    </>
  );
}
