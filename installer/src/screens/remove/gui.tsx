import { Button, CheckRow, Meter } from "@/components/controls";
import { ClosingLine } from "@/components/closing";
import { Actions, Card, Heading, Lead, PathText, Small, Spacer } from "@/components/layout";
import { formatBytes } from "@/lib/format";
import type { Closing, DeleteMode, Deleted } from "@/types";
import "./styles.scss";

/** The libraries screen 6b confirmed, by typing; nothing is deleted until Remove Lumen. */
export interface LibraryChoice {
  paths: string[];
  names: string[];
  mode: DeleteMode;
}

interface Props {
  libraries: string[];
  dataBytes: number;
  /** `backups/pre-migration/` when it holds anything; always kept. */
  safetyCopies: string | null;
  removeData: boolean;
  phase: "confirm" | "removing" | "done";
  onToggleData: (v: boolean) => void;
  onRemove: () => void;
  onCancel: () => void;
  onFinish: () => void;
  /** A running Lumen closing first; Cancel then stops the wait. */
  closing?: Closing | null;
  /** Why the last removal stopped (Lumen is still installed). */
  error?: string | null;
  /** Opens screen 6b; absent, the row isn't shown. */
  onDeleteLibraries?: () => void;
  deleteChoice?: LibraryChoice | null;
  onUndoDelete?: () => void;
  /** What happened to each chosen library (done phase). */
  deleted?: Deleted[];
}

const listed = (names: string[]) => names.join(", ");

function choiceSentence(c: LibraryChoice): string {
  const how = c.mode === "recycle" ? "moved to the Recycle Bin" : "deleted for good";
  return c.names.length === 1 ? `${c.names[0]} will be ${how}.` : `${c.names.length} libraries will be ${how}: ${listed(c.names)}.`;
}

export function Remove({
  libraries,
  dataBytes,
  safetyCopies,
  removeData,
  phase,
  onToggleData,
  onRemove,
  onCancel,
  onFinish,
  closing = null,
  error = null,
  onDeleteLibraries,
  deleteChoice = null,
  onUndoDelete,
  deleted = [],
}: Props) {
  if (phase === "removing") {
    return (
      <>
        <Heading>Removing Lumen…</Heading>
        <Meter label="Removing Lumen" />
        <ClosingLine closing={closing} onCancel={onCancel} />
      </>
    );
  }
  if (phase === "done") {
    const nameOf = (path: string) => deleteChoice?.names[deleteChoice.paths.indexOf(path)] ?? path;
    const gone = deleted.filter((d) => !d.problem).map((d) => nameOf(d.path));
    const failed = deleted.filter((d) => d.problem);
    const how = deleteChoice?.mode === "erase" ? "deleted for good" : "moved to the Recycle Bin";
    return (
      <>
        <Heading>Lumen is removed.</Heading>
        {gone.length > 0 ? (
          <Lead>{gone.length === 1 ? `${gone[0]} was ${how}.` : `${listed(gone)} were ${how}.`}</Lead>
        ) : (
          deleted.length === 0 && <Lead>Your photos and libraries were not touched.</Lead>
        )}
        {failed.length > 0 && (
          <div role="alert" className="remove__error">
            {failed.map((d) => (
              <p key={d.path}>{`${nameOf(d.path)}: ${d.problem}`}</p>
            ))}
          </div>
        )}
        {safetyCopies && (
          <Card>
            <div className="remove__kept">
              <span>Its safety copies stay in</span>
              <PathText path={safetyCopies} max={64} />
            </div>
          </Card>
        )}
        <Actions>
          <Spacer />
          <Button variant="primary" onClick={onFinish}>Finish</Button>
        </Actions>
      </>
    );
  }
  const kept = deleteChoice ? libraries.filter((p) => !deleteChoice.paths.includes(p)) : libraries;
  return (
    <>
      <Heading>Remove Lumen from this PC?</Heading>
      <Card>
        <div className="remove__keep">
          <i className="remove__lock" aria-hidden />
          <div className="remove__keep-text">
            <div className="remove__keep-line">{deleteChoice ? "Your other libraries are not touched." : "Your photos and libraries are not touched."}</div>
            {kept.map((p) => (
              <Small key={p}>
                <PathText path={p} max={72} />
              </Small>
            ))}
            {safetyCopies && (
              <Small>
                {"Safety copies of your catalogues are kept in "}
                <br />
                <PathText path={safetyCopies} max={72} />
              </Small>
            )}
          </div>
        </div>
      </Card>
      <CheckRow
        label={`Also remove Lumen's settings, caches and downloaded models (${formatBytes(dataBytes)})`}
        detail="Lumen then starts fresh if you install it again. Library folders are skipped even here."
        checked={removeData}
        onChange={onToggleData}
      />
      {deleteChoice ? (
        <div className="remove__doomed">
          <p role="alert">{choiceSentence(deleteChoice)}</p>
          <Button variant="link" onClick={onUndoDelete}>Undo</Button>
        </div>
      ) : (
        onDeleteLibraries &&
        libraries.length > 0 && (
          <div className="remove__libraries">
            <span>Want your libraries gone too?</span>
            <Button variant="link" className="remove__delete" onClick={onDeleteLibraries}>Delete libraries…</Button>
          </div>
        )
      )}
      {error && <p role="alert" className="remove__error">{error}</p>}
      <Actions>
        <Button variant="link" onClick={onCancel}>Cancel</Button>
        <Spacer />
        <Button variant="danger" onClick={onRemove}>Remove Lumen</Button>
      </Actions>
    </>
  );
}
