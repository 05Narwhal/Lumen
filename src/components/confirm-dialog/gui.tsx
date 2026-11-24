import { Button, Modal, TextInput } from "@mantine/core";
import { useState } from "react";
import { createStore } from "@/lib/store";
import "./styles.scss";

export interface ConfirmOptions {
  title: string;
  body?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  /** Focus the cancel button: the safe choice is to not do it. */
  focusCancel?: boolean;
}

interface Pending extends ConfirmOptions {
  resolve: (ok: boolean) => void;
  /** A name to type (`promptName`): the field's label and what it starts with. */
  field?: { label: string; value: string };
}

const store = createStore<{ pending: Pending | null }>({ pending: null });

/**
 * Asks for a name (Rename… from a context menu, Round 4G) in the same dialog: resolves to the
 * trimmed name, or `null` when cancelled, left empty or unchanged.
 */
export function promptName(opts: { title: string; label?: string; initial: string; confirmLabel?: string }): Promise<string | null> {
  return new Promise((resolve) => {
    const field = { label: opts.label ?? "Name", value: opts.initial };
    store.set({
      pending: {
        title: opts.title,
        confirmLabel: opts.confirmLabel ?? "Rename",
        field,
        resolve: (ok) => {
          const name = field.value.trim();
          resolve(ok && name && name !== opts.initial.trim() ? name : null);
        },
      },
    });
  });
}

function NameField({ field, onSubmit }: { field: { label: string; value: string }; onSubmit: () => void }) {
  const [value, setValue] = useState(field.value);
  return (
    <TextInput
      label={field.label}
      value={value}
      data-autofocus
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => {
        field.value = e.currentTarget.value;
        setValue(e.currentTarget.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onSubmit();
        }
      }}
    />
  );
}

/**
 * The only confirmation dialog in the app: reserved for irreversible actions
 * (Delete now, Empty trash, Delete library, delete a non-empty album) and for closing
 * while an import runs.
 */
export function confirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => store.set({ pending: { ...opts, resolve } }));
}

function settle(ok: boolean) {
  const p = store.get().pending;
  store.set({ pending: null });
  p?.resolve(ok);
}

/** Mount once. */
export function ConfirmDialogHost() {
  const { pending } = store.use();
  return (
    <Modal
      opened={pending !== null}
      onClose={() => settle(false)}
      data-part="confirm"
      title={pending?.title}
      centered
      size="sm"
      classNames={{ content: "confirm-dialog", header: "confirm-dialog__header", body: "confirm-dialog__body" }}
    >
      {pending?.body && <p className="confirm-dialog__text">{pending.body}</p>}
      {pending?.field && <NameField field={pending.field} onSubmit={() => settle(true)} />}
      <div className="confirm-dialog__actions">
        <Button variant={pending?.focusCancel ? "filled" : "default"} onClick={() => settle(false)} data-autofocus={pending?.focusCancel || undefined}>
          {pending?.cancelLabel ?? "Cancel"}
        </Button>
        <Button
          variant={pending?.focusCancel ? "default" : "filled"}
          color={pending?.danger ? "red" : undefined}
          onClick={() => settle(true)}
          data-autofocus={(!pending?.focusCancel && !pending?.field) || undefined}
        >
          {pending?.confirmLabel ?? "Confirm"}
        </Button>
      </div>
    </Modal>
  );
}
