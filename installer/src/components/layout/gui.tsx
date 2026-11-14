import type { HTMLAttributes, ReactNode } from "react";
import { shortPath } from "@/lib/format";
import "./styles.scss";

const cx = (...c: (string | false | undefined)[]) => c.filter(Boolean).join(" ");

export const Heading = ({ children }: { children: ReactNode }) => <h1 className="s-h">{children}</h1>;
export const Lead = ({ children }: { children: ReactNode }) => <p className="s-lead">{children}</p>;
export const Small = ({ children, ...rest }: { children: ReactNode } & HTMLAttributes<HTMLElement>) => (
  <p {...rest} className={cx("s-small", rest.className)}>{children}</p>
);
export const Card = ({ children, className }: { children: ReactNode; className?: string }) => (
  <div className={cx("s-card", className)}>{children}</div>
);
export const Actions = ({ children }: { children: ReactNode }) => <div className="s-actions">{children}</div>;
/** A folder path on one line, shortened in the middle; the whole path on hover. */
export const PathText = ({ path, max }: { path: string; max?: number }) => (
  <span className="s-path" title={path}>
    {shortPath(path, max)}
  </span>
);
/** Pushes the following controls to the right edge of an Actions bar. */
export const Spacer = () => <span className="s-spacer" />;
