import { useId, type ButtonHTMLAttributes, type ReactNode } from "react";
import "./styles.scss";

type Variant = "default" | "primary" | "link" | "danger";

export function Button({ variant = "default", className, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return <button type="button" {...rest} className={`s-btn s-btn--${variant}${className ? ` ${className}` : ""}`} />;
}

/** A switch row: label, optional detail under it, the track on the right. */
export function SwitchRow({ label, detail, checked, onChange }: { label: string; detail?: ReactNode; checked: boolean; onChange: (v: boolean) => void }) {
  const id = useId();
  return (
    <div className="s-row">
      <b id={`${id}-l`}>{label}</b>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-labelledby={`${id}-l`}
        aria-describedby={detail ? `${id}-d` : undefined}
        className="s-switch"
        onClick={() => onChange(!checked)}
      />
      {detail && <span id={`${id}-d`} className="s-row__detail">{detail}</span>}
    </div>
  );
}

/** A checkbox in a card-like label; `detail` sits under the label. */
export function CheckRow({ label, detail, checked, onChange }: { label: string; detail?: ReactNode; checked: boolean; onChange: (v: boolean) => void }) {
  const id = useId();
  return (
    <label className="s-check" htmlFor={id}>
      <input id={id} type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <span className="s-check__label">{label}</span>
        {detail && <span className="s-check__detail">{detail}</span>}
      </span>
    </label>
  );
}

/** A progress bar; no `value` is an indeterminate sweep. */
export function Meter({ value, label }: { value?: number; label: string }) {
  const known = value !== undefined;
  return (
    <div
      className={known ? "s-meter" : "s-meter s-meter--indeterminate"}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={known ? Math.round(value * 100) : undefined}
    >
      <i style={known ? { width: `${Math.round(Math.min(1, Math.max(0, value)) * 100)}%` } : undefined} />
    </div>
  );
}
