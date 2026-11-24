import { Button } from "@mantine/core";
import { useUpdateStatus } from "@/hooks/use-updates";
import "./styles.scss";

/**
 * A security fix is staged and Lumen did not take it at launch (the switch is off, another Lumen
 * was open, or the installer did not start): it stays here, above the content, until it installs
 * at Close or the person chooses *Install now*. Never a modal, and never for an ordinary update.
 */
export function SecurityBanner() {
  const { status, installNow } = useUpdateStatus();
  if (!status?.security) return null;
  return (
    <div className="security-banner" data-part="statusbar" role="status">
      <span className="security-banner__text">A security fix is ready</span>
      <Button size="compact-sm" onClick={() => void installNow()}>
        Install now
      </Button>
    </div>
  );
}
