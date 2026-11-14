import { Navigate, useParams } from "react-router";
import { SlideshowEditor } from "./editor";
import "./styles.scss";

/**
 * `/slideshows/:id` (UX spec §6.6). The list is the Memories page's Slideshows tab (Round 20);
 * the router sends `/slideshows` there, and so does anything that is not an id.
 */
export function SlideshowsRoute() {
  const { id } = useParams();
  if (id && /^\d+$/.test(id)) return <SlideshowEditor key={id} id={Number(id)} />;
  return <Navigate to="/memories?tab=slideshows" replace />;
}
