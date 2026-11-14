import type { ReactNode } from "react";
import "./styles.scss";

/** A short state beside a control's name ("In development", Round 14). Not a button. */
export function StatusBadge({ children }: { children: ReactNode }) {
  return <span className="status-badge">{children}</span>;
}
