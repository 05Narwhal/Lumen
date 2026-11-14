import { useParams } from "react-router";
import { MemoryDetail } from "./detail";
import { MemoriesIndex } from "./index-page";
import "./styles.scss";

/** `/memories` and `/memories/:id` (UX spec §6.5). */
export function MemoriesRoute() {
  const { id } = useParams();
  if (id && /^\d+$/.test(id)) return <MemoryDetail key={id} id={Number(id)} />;
  return <MemoriesIndex />;
}
