import { Button } from "@mantine/core";
import "./styles.scss";

/** The Develop screen a person was on, by its route: `/develop` or `/develop/<id>`, query allowed. */
export const isDevelopRoute = (route: string | null | undefined) => !!route && /^\/develop(?:[/?#]|$)/.test(route);

/**
 * Shown on Home after Lumen relaunched following a crash, a watchdog restart or *Restart Lumen*
 * (Round 24 §6.2). Develop's drafts are saved as they are made, so the message can promise the
 * edits are kept; *Back to Develop* is offered only when that is where the person was.
 */
export function RestartAlert({ route, onBack, onDismiss }: { route: string | null; onBack: (route: string) => void; onDismiss: () => void }) {
  return (
    <div className="restart-alert" role="alert">
      <span className="restart-alert__text">Lumen restarted after a problem. Your edits are kept.</span>
      {route && isDevelopRoute(route) && (
        <Button size="compact-sm" onClick={() => onBack(route)}>
          Back to Develop
        </Button>
      )}
      <Button size="compact-sm" variant="default" onClick={onDismiss}>
        Dismiss
      </Button>
    </div>
  );
}
