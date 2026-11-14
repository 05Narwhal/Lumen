import { Button, Modal, PasswordInput } from "@mantine/core";
import { IconLock } from "@tabler/icons-react";
import { useEffect, useState, type FormEvent } from "react";
import { invoke, isAppError } from "@/lib/ipc";

/** The shortest password accepted (`photos_core::vault::MIN_PASSWORD`). */
const MIN_PASSWORD = 4;
/** How long the forgotten-password warning makes you wait before it lets you continue. */
export const RESET_WAIT_S = 10;

/** Choosing a password: the first time, or after *Forgot password?*. */
function ChoosePassword({ reset, onDone }: { reset: boolean; onDone: () => void }) {
  const [first, setFirst] = useState("");
  const [again, setAgain] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (first.length < MIN_PASSWORD) return setError(`Use at least ${MIN_PASSWORD} characters.`);
    if (first !== again) return setError("The passwords do not match.");
    setBusy(true);
    try {
      await invoke(reset ? "vault_reset_password" : "vault_set_password", { password: first });
      onDone();
    } catch (err) {
      setError(isAppError(err) ? err.message : "Could not set the password.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="hidden-route__lock" onSubmit={submit}>
      <IconLock size={32} className="hidden-route__lock-icon" aria-hidden />
      <h2>{reset ? "Choose a new password" : "Choose a password for Hidden"}</h2>
      <p>Photos here don't appear anywhere else in Lumen. The password keeps them out of sight in the app.</p>
      <PasswordInput label="Password" value={first} onChange={(e) => setFirst(e.currentTarget.value)} autoFocus />
      <PasswordInput label="The same password again" value={again} onChange={(e) => setAgain(e.currentTarget.value)} />
      {error && (
        <p className="hidden-route__error" role="alert">
          {error}
        </p>
      )}
      <Button type="submit" loading={busy}>
        Set password
      </Button>
    </form>
  );
}

/** The forgotten-password warning: anyone at this computer could do this, so it makes you wait. */
function ForgotDialog({ onClose, onContinue }: { onClose: () => void; onContinue: () => void }) {
  // One deadline, read on a short interval: a count of chained timeouts drifts and stalls.
  const [deadline] = useState(() => Date.now() + RESET_WAIT_S * 1000);
  const [left, setLeft] = useState(RESET_WAIT_S);
  useEffect(() => {
    const tick = () => setLeft(Math.max(0, Math.ceil((deadline - Date.now()) / 1000)));
    const t = setInterval(tick, 250);
    return () => clearInterval(t);
  }, [deadline]);
  return (
    <Modal opened onClose={onClose} title="Forgot the password?" centered size="sm">
      <div className="hidden-route__forgot">
        <p>You can choose a new one. Anyone using this computer could do the same, so the Hidden folder keeps photos out of sight rather than locked away. Your hidden photos stay hidden.</p>
        <div className="hidden-route__forgot-actions">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button color="red" disabled={left > 0} onClick={onContinue}>
            {left > 0 ? `Choose a new password (${left})` : "Choose a new password"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

/** Unlocking, with the way out for a forgotten password. */
function Unlock({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [forgot, setForgot] = useState(false);
  const [resetting, setResetting] = useState(false);
  if (resetting) return <ChoosePassword reset onDone={onDone} />;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    try {
      if (await invoke<boolean>("vault_unlock", { password })) onDone();
      else setError("That is not the password.");
    } catch (err) {
      setError(isAppError(err) ? err.message : "Could not open Hidden.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="hidden-route__lock" onSubmit={submit}>
      <IconLock size={32} className="hidden-route__lock-icon" aria-hidden />
      <h2>Hidden is locked</h2>
      <PasswordInput label="Password" value={password} onChange={(e) => setPassword(e.currentTarget.value)} autoFocus />
      {error && (
        <p className="hidden-route__error" role="alert">
          {error}
        </p>
      )}
      <Button type="submit" loading={busy}>
        Unlock
      </Button>
      <Button variant="subtle" size="xs" onClick={() => setForgot(true)}>
        Forgot password?
      </Button>
      {forgot && (
        <ForgotDialog
          onClose={() => setForgot(false)}
          onContinue={() => {
            setForgot(false);
            setResetting(true);
          }}
        />
      )}
    </form>
  );
}

export function VaultLockScreen({ hasPassword, onOpened }: { hasPassword: boolean; onOpened: () => void }) {
  return hasPassword ? <Unlock onDone={onOpened} /> : <ChoosePassword reset={false} onDone={onOpened} />;
}
