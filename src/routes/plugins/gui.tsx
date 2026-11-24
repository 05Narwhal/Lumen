import { Button } from "@mantine/core";
import { IconPlugConnectedX, IconUpload } from "@tabler/icons-react";
import { useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { Segmented, type SegmentedOption } from "@/components/segmented";
import { useStoreIndex } from "@/hooks/use-store-index";
import { usePluginActions } from "./actions";
import { openLink } from "./detail";
import { Discover } from "./discover";
import { Installed } from "./installed";
import { Themes } from "./themes";
import "./styles.scss";

type Tab = "plugins" | "themes" | "installed" | "upload";
const TABS: SegmentedOption<Tab>[] = [
  { label: "Plugins", value: "plugins" },
  { label: "Themes", value: "themes" },
  { label: "Installed", value: "installed" },
  { label: "Upload", value: "upload" },
];
const REPO = "https://github.com/05Narwhal/lumen-plugins";

/** `/plugins`: the plugin store (Round 26 direction B). `?tab=themes|installed|upload`, absent meaning Plugins; `?see=<section>` is a section's *See all*. */
export function PluginsRoute() {
  const [params, setParams] = useSearchParams();
  const raw = params.get("tab");
  const tab: Tab = raw === "installed" || raw === "upload" || raw === "themes" ? raw : "plugins";
  const setTab = (t: Tab) => setParams(t === "plugins" ? {} : { tab: t }, { replace: true });
  const { state, reload: reloadIndex } = useStoreIndex();
  const { plugins, status, updates, busyId, actionOf, run, act, menuOf } = usePluginActions();

  const updateAll = async () => {
    for (const u of updates) await run(u.id, true);
  };

  const offlineBanner = status?.offline && state.phase === "ready";
  let body;
  if (tab === "upload") {
    body = (
      <EmptyState
        icon={<IconUpload />}
        title="Uploading from Lumen is coming later."
        description="For now, open a pull request."
        action={
          <div className="store-upload">
            <Button variant="default" disabled>
              Upload a plugin
            </Button>
            <Button variant="subtle" onClick={() => openLink(REPO)}>
              {REPO}
            </Button>
          </div>
        }
      />
    );
  } else if (tab === "installed") {
    body = <Installed plugins={plugins} updates={updates} autoUpdate={status?.autoUpdate ?? true} busyId={busyId} onUpdate={(id) => void run(id, true)} onUpdateAll={() => void updateAll()} menuOf={menuOf} onDiscover={() => setTab("plugins")} />;
  } else if (state.phase === "offline") {
    body = <EmptyState icon={<IconPlugConnectedX />} title="You're offline." description="The plugin store needs the internet." action={<Button variant="default" onClick={reloadIndex}>Try again</Button>} />;
  } else if (state.phase === "failed") {
    body = <EmptyState icon={<IconPlugConnectedX />} title="The store could not be loaded." description={state.message} action={<Button variant="default" onClick={reloadIndex}>Try again</Button>} />;
  } else if (state.phase === "ready" && tab === "themes") {
    body = <Themes themes={state.index.themes ?? []} featured={state.index.featuredTheme} />;
  } else if (state.phase === "ready") {
    body = <Discover entries={state.index.plugins} featured={state.index.featured} actionOf={actionOf} busyId={busyId} onAction={act} menuOf={menuOf} />;
  }

  return (
    <div className="plugins-route">
      <PageHeader title={tab === "themes" ? "Themes" : "Plugins"} search={false} toolbar={<Segmented label="Show" size="sm" data={TABS} value={tab} onChange={setTab} />} />
      {offlineBanner && (tab === "plugins" || tab === "themes") && (
        <div className="store-banner" role="status">
          <span>You're offline. Showing the last list.</span>
          <Button size="xs" variant="default" onClick={reloadIndex}>
            Try again
          </Button>
        </div>
      )}
      <div className="plugins-route__body">{body}</div>
    </div>
  );
}
