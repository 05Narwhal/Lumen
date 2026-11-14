import { useParams } from "react-router";
import { PersonDetail } from "./detail";
import { PeopleAndPets } from "./people-and-pets";
import "./styles.scss";

/** `/people` and `/people/:id` (Phase 7b). */
export function PeopleRoute() {
  const { id } = useParams();
  if (id && /^\d+$/.test(id)) return <PersonDetail key={id} id={Number(id)} />;
  return <PeopleAndPets />;
}
