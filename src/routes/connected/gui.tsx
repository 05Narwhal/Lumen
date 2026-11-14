import { useParams } from "react-router";
import { PageHeader } from "@/components/page-header";
import { DeviceRoute } from "./device";
import { LibraryCard, libraryName } from "./library-card";
import { MacDiskRoute } from "./mac-disk";
import "./styles.scss";

/** An Apple Photos library picked on this computer (Import → *Apple Photos library…*). */
function LibraryRoute({ path }: { path: string }) {
  return (
    <div className="connected-route">
      <PageHeader title={libraryName(path)} search={false} />
      <div className="connected-route__body">
        <LibraryCard disk={null} path={path} />
      </div>
    </div>
  );
}

/**
 * `/connected/:kind/:id` — what a connected source holds before it is imported (UX spec
 * §6.7a). `volume` and `iphone` are cards, drives and phones; `mac` is a Mac-formatted
 * disk; `photos` is an Apple Photos library by path.
 */
export function ConnectedRoute() {
  const { kind = "", id = "" } = useParams();
  if (kind === "mac") return <MacDiskRoute key={id} id={id} />;
  if (kind === "photos") return <LibraryRoute key={id} path={id} />;
  return <DeviceRoute key={`${kind}:${id}`} kind={kind} id={id} />;
}
