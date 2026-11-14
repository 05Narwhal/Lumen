import type { ReactNode } from "react";
import "./styles.scss";

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state" role="status">
      <div className="empty-state__icon" aria-hidden>
        {icon}
      </div>
      <h2 className="empty-state__title">{title}</h2>
      <p className="empty-state__desc">{description}</p>
      {action && <div className="empty-state__action">{action}</div>}
    </div>
  );
}
