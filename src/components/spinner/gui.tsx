import "./styles.scss";

/** Themed indeterminate spinner. Sizes: sm (16 px) for inline, md (28 px) for panels. */
export function Spinner({ size = "sm", label = "Loading" }: { size?: "sm" | "md"; label?: string }) {
  const px = size === "sm" ? 16 : 28;
  const r = px / 2 - 2;
  const c = 2 * Math.PI * r;
  return (
    <span className="spinner" role="status" aria-label={label} data-size={size} style={{ width: px, height: px }}>
      <svg width={px} height={px} viewBox={`0 0 ${px} ${px}`} aria-hidden>
        <circle className="spinner__track" cx={px / 2} cy={px / 2} r={r} />
        <circle className="spinner__arc" cx={px / 2} cy={px / 2} r={r} strokeDasharray={`${c * 0.72} ${c}`} />
      </svg>
    </span>
  );
}
