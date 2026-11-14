import { Button, Checkbox, Modal, TextInput } from "@mantine/core";
import { useEffect, useState } from "react";
import { PANELS } from "@/lib/develop";
import type { Panel } from "@/types/generated/Panel";
import "./styles.scss";

/**
 * Which panels an action carries: *Save preset…* (with a name) and *Copy settings…*. Every
 * panel starts checked, as Lightroom's dialogs do — except *Masks* (Phase 8e), offered only
 * when `masks` is set and unticked, because masks belong to one photo's pixels.
 */
export function PanelChoice({
  opened,
  title,
  confirmLabel,
  name,
  masks = false,
  onConfirm,
  onClose,
}: {
  opened: boolean;
  title: string;
  confirmLabel: string;
  /** A name field with this starting value; omitted, no name is asked for. */
  name?: string;
  /** Offer *Masks* too (copying settings; never a preset). */
  masks?: boolean;
  onConfirm: (choice: { name: string; panels: Panel[] }) => void;
  onClose: () => void;
}) {
  const [chosen, setChosen] = useState<Panel[]>(PANELS.map((p) => p.value));
  const [text, setText] = useState(name ?? "");
  useEffect(() => {
    if (opened) {
      setChosen(PANELS.map((p) => p.value));
      setText(name ?? "");
    }
  }, [opened, name]);
  const needsName = name !== undefined;
  const ready = chosen.length > 0 && (!needsName || text.trim().length > 0);
  const confirm = () => {
    if (!ready) return;
    onConfirm({ name: text.trim(), panels: [...PANELS.map((p) => p.value), "masks" as Panel].filter((p) => chosen.includes(p)) });
  };
  return (
    <Modal opened={opened} onClose={onClose} title={title} centered size="sm">
      <form
        className="panel-choice"
        onSubmit={(e) => {
          e.preventDefault();
          confirm();
        }}
      >
        {needsName && <TextInput label="Name" value={text} onChange={(e) => setText(e.currentTarget.value)} data-autofocus />}
        <Checkbox.Group label="Panels" value={chosen} onChange={(v) => setChosen(v as Panel[])}>
          <div className="panel-choice__list">
            {PANELS.map((p) => (
              <Checkbox key={p.value} value={p.value} label={p.label} size="sm" />
            ))}
            {masks && <Checkbox value="masks" label="Masks" size="sm" />}
          </div>
        </Checkbox.Group>
        <div className="panel-choice__actions">
          <Button variant="subtle" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!ready}>
            {confirmLabel}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
