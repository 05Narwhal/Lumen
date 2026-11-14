import { Button, TextInput } from "@mantine/core";
import { IconRotate2, IconSearch } from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import { useSettings } from "@/hooks/use-settings";
import { defaultKeys, eventToKey, findConflict, formatKey, keybindGroups, keysFor, sameKey, type KeyAction, type KeyOverrides } from "@/lib/keybinds";
import "./styles.scss";

interface Clash {
  id: string;
  key: string;
  other: KeyAction;
}

/** Every shortcut in the app, by where it works: find one, press new keys for it, reset it. */
export function KeybindList() {
  const { settings, update } = useSettings();
  const overrides: KeyOverrides = settings?.keybinds ?? {};
  const [filter, setFilter] = useState("");
  const [recording, setRecording] = useState<string | null>(null);
  const [clash, setClash] = useState<Clash | null>(null);

  const change = (f: (o: KeyOverrides) => KeyOverrides) => update((s) => ({ ...s, keybinds: f(s.keybinds ?? {}) }));
  /** Sets an action's keys; keys equal to its defaults drop the override. */
  const withKeys = (o: KeyOverrides, id: string, keys: string[]): KeyOverrides => {
    const next = { ...o };
    const def = defaultKeys(id);
    if (keys.length === def.length && keys.every((k, i) => sameKey(k, def[i]))) delete next[id];
    else next[id] = keys;
    return next;
  };

  useEffect(() => {
    if (recording === null) return;
    const id = recording;
    // Captured before the app's own hotkeys, so the key being recorded does nothing else.
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const bare = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
      if (e.key === "Escape" && bare) return setRecording(null);
      if (e.key === "Backspace" && bare) {
        change((o) => withKeys(o, id, []));
        return setRecording(null);
      }
      const key = eventToKey(e);
      if (!key) return; // a modifier on its own: keep listening
      setRecording(null);
      const other = findConflict(id, key, overrides);
      if (other) setClash({ id, key, other });
      else change((o) => withKeys(o, id, [key]));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // `change` and `withKeys` only close over `update`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording, overrides]);

  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return keybindGroups()
      .map((g) => ({
        ...g,
        binds: q
          ? g.binds.filter((b) => b.action.toLowerCase().includes(q) || keysFor(b.id, overrides).some((k) => k.toLowerCase() === q || k.toLowerCase().includes(`+${q}`)))
          : g.binds,
      }))
      .filter((g) => g.binds.length > 0);
  }, [filter, overrides]);

  const replace = (c: Clash) => {
    change((o) => {
      const left = keysFor(c.other.id, o).filter((k) => !sameKey(k, c.key));
      return withKeys(withKeys(o, c.other.id, left), c.id, [c.key]);
    });
    setClash(null);
  };

  const keycaps = (keys: string[]) =>
    keys.map((k, i) => (
      <span key={k} className="keybind-list__keys">
        {i > 0 && <span className="keybind-list__or">or</span>}
        {formatKey(k).map((part) => (
          <kbd key={part}>{part}</kbd>
        ))}
      </span>
    ));

  return (
    <div className="keybind-list">
      <div className="keybind-list__bar">
        <TextInput
          className="keybind-list__filter"
          size="xs"
          placeholder="Find a shortcut"
          aria-label="Find a shortcut"
          leftSection={<IconSearch size={14} />}
          value={filter}
          onChange={(e) => setFilter(e.currentTarget.value)}
        />
        {Object.keys(overrides).length > 0 && (
          <Button size="xs" variant="subtle" onClick={() => change(() => ({}))}>
            Reset all
          </Button>
        )}
      </div>
      {groups.length === 0 && <p className="keybind-list__empty">No shortcut matches “{filter}”.</p>}
      <div className="keybind-list__groups">
        {groups.map((g) => (
          <section key={g.id} className="keybind-list__group" aria-label={g.title}>
            <h3>{g.title}</h3>
            <p className="keybind-list__where">{g.where}</p>
            <dl>
              {g.binds.map((b) => {
                const keys = keysFor(b.id, overrides);
                const changed = b.id in overrides;
                return (
                  <div key={b.id} className="keybind-list__row">
                    <dt>{b.action}</dt>
                    <dd>
                      {b.fixed ? (
                        <span className="keybind-list__fixed" title="This one cannot be changed">
                          {keycaps(keys)}
                        </span>
                      ) : (
                        <>
                          {changed && (
                            <button type="button" className="keybind-list__reset" aria-label={`Reset ${b.action}`} title="Back to the default" onClick={() => change((o) => withKeys(o, b.id, defaultKeys(b.id)))}>
                              <IconRotate2 size={14} />
                            </button>
                          )}
                          <button
                            type="button"
                            className="keybind-list__change"
                            data-recording={recording === b.id || undefined}
                            aria-label={`Change the keys for ${b.action}`}
                            onClick={() => {
                              setClash(null);
                              setRecording(recording === b.id ? null : b.id);
                            }}
                          >
                            {recording === b.id ? <span className="keybind-list__hint">Press keys…</span> : keys.length === 0 ? <span className="keybind-list__hint">Not set</span> : keycaps(keys)}
                          </button>
                        </>
                      )}
                    </dd>
                    {clash?.id === b.id && (
                      <p className="keybind-list__clash" role="alert">
                        <span>
                          {formatKey(clash.key).join("+")} is already used by {clash.other.action}.
                        </span>
                        <Button size="compact-xs" variant="light" onClick={() => replace(clash)}>
                          Replace
                        </Button>
                        <Button size="compact-xs" variant="subtle" onClick={() => setClash(null)}>
                          Cancel
                        </Button>
                      </p>
                    )}
                  </div>
                );
              })}
            </dl>
          </section>
        ))}
      </div>
    </div>
  );
}
