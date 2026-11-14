import { useId, useMemo, useState } from "react";
import { Button } from "@/components/controls";
import { Actions, Card, Heading, PathText, Small, Spacer } from "@/components/layout";
import { formatBytes } from "@/lib/format";
import type { BinCheck, LibraryDetail, Refusal } from "@/types";
import "./styles.scss";

/** The word that confirms deleting several libraries at once (case-sensitive). */
export const CONFIRM_WORD = "DELETE";

const REFUSAL: Record<Refusal, string> = {
  "not-a-library": "This folder no longer holds a Lumen library.",
  "too-high": "This library is a whole drive or user folder. Delete it yourself if you mean to.",
  "holds-lumen": "Lumen's own files are inside this library. Delete it yourself after removing Lumen.",
  link: "This library's folder is a link to somewhere else. Delete it yourself if you mean to.",
};

interface Props {
  /** null while Setup counts the photos. */
  libraries: LibraryDetail[] | null;
  /** The Recycle Bin check is running. */
  checking: boolean;
  /** Set when the chosen libraries don't fit the Recycle Bin: Setup asks before erasing. */
  tooBig: BinCheck | null;
  /** The person typed the confirmation; Setup then checks the Recycle Bin. */
  onConfirm: (paths: string[]) => void;
  onEraseAnyway: () => void;
  onCancelTooBig: () => void;
  onBack: () => void;
}

const counted = (l: LibraryDetail) => `${l.photos.toLocaleString("en-US")} ${l.photos === 1 ? "photo" : "photos"} · ${formatBytes(l.bytes)}`;

export function DeleteLibraries({ libraries, checking, tooBig, onConfirm, onEraseAnyway, onCancelTooBig, onBack }: Props) {
  const fieldId = useId();
  const [typed, setTyped] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const deletable = useMemo(() => (libraries ?? []).filter((l) => !l.refused), [libraries]);
  const one = libraries?.length === 1 ? deletable[0] : undefined;
  const chosen = one ? [one.path] : picked;
  const want = one ? one.name : CONFIRM_WORD;
  const ready = chosen.length > 0 && typed === want && !checking;
  const chosenBytes = deletable.filter((l) => chosen.includes(l.path)).reduce((n, l) => n + l.bytes, 0);
  const plural = chosen.length > 1 || (!one && chosen.length !== 1);

  if (!libraries) {
    return (
      <>
        <Heading>Delete libraries</Heading>
        <p role="status" className="dl__status">Counting photos…</p>
      </>
    );
  }

  if (tooBig) {
    const these = chosen.length === 1 ? "This library" : "These libraries";
    const sentence =
      tooBig.room === 0
        ? `${these} can't go to the Recycle Bin on this drive. ${chosen.length === 1 ? "It" : "They"} would be deleted for good.`
        : `${these} ${chosen.length === 1 ? "is" : "are"} too big for the Recycle Bin (${formatBytes(tooBig.bytes)}; it has room for ${formatBytes(tooBig.room)}). ${chosen.length === 1 ? "It" : "They"} would be deleted for good.`;
    return (
      <>
        <Heading>Delete for good?</Heading>
        <Card className="dl__warn">
          <p role="alert">{sentence}</p>
          <Small>This can't be undone. Anything else you keep in those folders stays.</Small>
        </Card>
        <Actions>
          <Button variant="link" onClick={onCancelTooBig}>Cancel</Button>
          <Spacer />
          <Button variant="danger" onClick={onEraseAnyway}>Delete for good</Button>
        </Actions>
      </>
    );
  }

  if (deletable.length === 0) {
    return (
      <>
        <Heading>{libraries.length === 1 ? `Delete ${libraries[0].name}?` : "Delete libraries"}</Heading>
        <Card>
          <ul className="dl__list">
            {libraries.map((l) => (
              <li key={l.path} className="dl__row dl__row--off">
                <span className="dl__text">
                  <b>{l.name}</b>
                  <PathText path={l.path} max={64} />
                  {l.refused && <span className="dl__why">{REFUSAL[l.refused]}</span>}
                </span>
              </li>
            ))}
          </ul>
        </Card>
        <Actions>
          <Button variant="link" onClick={onBack}>Back</Button>
        </Actions>
      </>
    );
  }

  const toggle = (path: string) => setPicked((p) => (p.includes(path) ? p.filter((x) => x !== path) : [...p, path]));
  const all = picked.length === deletable.length;

  return (
    <>
      <Heading>{one ? `Delete ${one.name}?` : "Delete libraries"}</Heading>
      {one ? (
        <Card className="dl__one">
          <b>{one.name}</b>
          <PathText path={one.path} max={72} />
          <span className="dl__count">{counted(one)}</span>
        </Card>
      ) : (
        <Card className="dl__many">
          <div className="dl__head">
            <span>{picked.length === 0 ? "None selected" : `${picked.length} of ${deletable.length} · ${formatBytes(chosenBytes)}`}</span>
            <Button variant="link" onClick={() => setPicked(all ? [] : deletable.map((l) => l.path))}>
              {all ? "Select none" : "Select all"}
            </Button>
          </div>
          <ul className="dl__list">
            {libraries.map((l) =>
              l.refused ? (
                <li key={l.path} className="dl__row dl__row--off">
                  <span className="dl__box" aria-hidden />
                  <span className="dl__text">
                    <b>{l.name}</b>
                    <span className="dl__why">{REFUSAL[l.refused]}</span>
                  </span>
                </li>
              ) : (
                <li key={l.path} className="dl__row">
                  <label className="dl__pick">
                    <input type="checkbox" checked={picked.includes(l.path)} onChange={() => toggle(l.path)} />
                    <span className="dl__text">
                      <b>{l.name}</b>
                      <PathText path={l.path} max={60} />
                    </span>
                    <span className="dl__count">{counted(l)}</span>
                  </label>
                </li>
              ),
            )}
          </ul>
        </Card>
      )}
      <Small>Lumen's files in {plural ? "these folders" : "this folder"} go to the Recycle Bin. Anything else you keep there stays.</Small>
      <div className="dl__confirm">
        <label htmlFor={fieldId}>{one ? `Type "${one.name}" to delete it` : `Type ${CONFIRM_WORD} to delete them`}</label>
        <input
          id={fieldId}
          type="text"
          autoComplete="off"
          spellCheck={false}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && ready) onConfirm(chosen);
          }}
        />
      </div>
      <Actions>
        <Button variant="link" onClick={onBack}>Back</Button>
        <Spacer />
        <Button variant="danger" disabled={!ready} onClick={() => onConfirm(chosen)}>
          {one ? "Delete library" : chosen.length === 0 ? "Delete libraries" : `Delete ${chosen.length} ${chosen.length === 1 ? "library" : "libraries"}`}
        </Button>
      </Actions>
    </>
  );
}
