import { Button } from "@mantine/core";
import { IconListCheck } from "@tabler/icons-react";
import type { ComponentType } from "react";
import { useNavigate, useParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { isProposalKind } from "@/lib/proposals";
import { PlaceReview } from "./places";
import { ProposalReview } from "./proposals";
import "./styles.scss";

/**
 * The review pages a plugin's `open-review:<kind>` or tool answer can open. Each kind reads the
 * plugin from `?plugin=<id>`; nothing here knows which plugin is asking. The kinds a plugin
 * submits through `proposals.submit` (a place, a date, tags, a label, a name, a title or a caption)
 * share one page, `ProposalReview`; `places` is the place review with its groups and moments.
 */
const KINDS: Record<string, ComponentType> = {
  places: PlaceReview,
};

/** `/review/:kind?plugin=<id>`: one route for every review a plugin feeds. */
export function ReviewRoute() {
  const { kind = "" } = useParams();
  const navigate = useNavigate();
  const Page = KINDS[kind];
  if (Page) return <Page />;
  if (isProposalKind(kind)) return <ProposalReview key={kind} kind={kind} />;
  return (
    <div className="review-page">
      <PageHeader title="Review" search={false} back={{ to: "/", label: "Back" }} />
      <EmptyState
        icon={<IconListCheck size={40} />}
        title="Nothing to review here."
        description="This review isn't one Lumen knows; the plugin may need a newer Lumen."
        action={
          <Button variant="light" onClick={() => navigate(-1)}>
            Back
          </Button>
        }
      />
    </div>
  );
}
