import { Button } from "@mantine/core";
import { IconPlugConnectedX } from "@tabler/icons-react";
import { Link } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { isAppError } from "@/lib/ipc";

/** A device that has gone, or a phone that has not trusted this computer yet. */
export function DeviceGone({ name, error, retry }: { name: string; error: unknown; retry: () => void }) {
  const trust = isAppError(error) && error.code === "iphone_trust_required";
  return (
    <div className="connected-route">
      <PageHeader title={name} search={false} />
      <EmptyState
        icon={<IconPlugConnectedX size={40} />}
        title={trust ? "Unlock the iPhone and tap Trust" : isAppError(error) ? error.message : "The device could not be read."}
        description={trust ? "The phone has to allow this computer before its photos can be shown." : "Connect it again, or import from somewhere else."}
        action={
          trust ? (
            <Button onClick={retry}>Try again</Button>
          ) : (
            <Button component={Link} to="/import">
              Go to Import
            </Button>
          )
        }
      />
    </div>
  );
}
