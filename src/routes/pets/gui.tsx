import { Navigate, useParams } from "react-router";
import { PetDetail } from "./detail";
import "./styles.scss";

/** `/pets/:id` (Round 4C). The list is People & Pets' Pets tab; `/pets` redirects there (Round 20). */
export function PetsRoute() {
  const { id } = useParams();
  if (id && /^\d+$/.test(id)) return <PetDetail key={id} id={Number(id)} />;
  return <Navigate to="/people?show=pets" replace />;
}
