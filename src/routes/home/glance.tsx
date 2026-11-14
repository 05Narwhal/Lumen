import { Button } from "@mantine/core";
import { Link } from "react-router";
import { MiniMap } from "@/components/mini-map";
import { useHomeSummary } from "@/hooks/use-home";
import { usePeople } from "@/hooks/use-people";
import { usePets } from "@/hooks/use-pets";
import { writeTags } from "@/lib/search-tags";
import type { PlaceSummary } from "@/types/generated/PlaceSummary";
import { mediaUrl } from "@/utils/media";

/** Round faces on Home: named people by photo count, then pets. */
const FACES = 10;

const photos = (n: number) => `${n.toLocaleString()} ${n === 1 ? "photo" : "photos"}`;

/** The Library narrowed to one place, the way the tag search writes it. */
function placeLink(p: PlaceSummary): string {
  const params = new URLSearchParams();
  writeTags(params, [{ label: p.label, value: { type: "place", place: p.place } }]);
  return `/library?${params}`;
}

function PeopleAndPets() {
  const { people } = usePeople();
  const { pets } = usePets();
  const faces = [
    ...people
      .filter((p) => p.name)
      .sort((a, b) => b.photoCount - a.photoCount)
      .map((p) => ({ key: `person:${p.id}`, to: `/people/${p.id}`, name: p.name ?? "", src: p.coverFaceId != null ? mediaUrl("face", p.coverFaceId) : null, pet: false })),
    ...pets
      .filter((p) => p.name)
      .sort((a, b) => b.photoCount - a.photoCount)
      .map((p) => ({ key: `pet:${p.id}`, to: `/pets/${p.id}`, name: p.name ?? "", src: p.coverDetectionId != null ? mediaUrl("pet", p.coverDetectionId) : null, pet: true })),
  ].slice(0, FACES);
  if (faces.length === 0) return null;
  return (
    <div className="home-route__half" aria-label="People & Pets" role="group">
      <header className="home-route__batch-head">
        <h2>People &amp; Pets</h2>
        <Button size="xs" variant="subtle" component={Link} to="/people">
          See all
        </Button>
      </header>
      <div className="home-route__faces">
        {faces.map((f) => (
          <Link key={f.key} to={f.to} className="home-route__face" title={f.name}>
            {f.src ? <img src={f.src} alt="" loading="lazy" decoding="async" data-pet={f.pet} /> : <span data-pet={f.pet} />}
            <span className="home-route__face-name">{f.name}</span>
          </Link>
        ))}
      </div>
    </div>
  );
}

function Places() {
  const { summary } = useHomeSummary();
  const places = (summary?.places ?? []).slice(0, 4);
  if (places.length === 0) return null;
  return (
    <div className="home-route__half" aria-label="Places" role="group">
      <header className="home-route__batch-head">
        <h2>Places</h2>
        <Button size="xs" variant="subtle" component={Link} to="/map">
          Open map
        </Button>
      </header>
      <div className="home-route__places">
        {places.map((p) => (
          <Link key={p.label} to={placeLink(p)} className="home-route__place">
            <span className="home-route__place-media">
              {p.lat != null && p.lon != null ? (
                <MiniMap lat={p.lat} lon={p.lon} width={200} height={110} zoom={5} />
              ) : p.coverAssetId != null ? (
                <img src={mediaUrl("thumb", p.coverAssetId)} alt="" loading="lazy" />
              ) : null}
            </span>
            <span className="home-route__face-name">{p.label}</span>
            <span className="home-route__card-sub">{photos(Number(p.photoCount))}</span>
          </Link>
        ))}
      </div>
    </div>
  );
}

/** Your library (Round 20 §10.4): faces and places, side by side from 1,100 px. Each half is absent when empty. */
export function LibraryGlance() {
  return (
    <section className="home-route__glance" aria-label="Your library">
      <PeopleAndPets />
      <Places />
    </section>
  );
}
