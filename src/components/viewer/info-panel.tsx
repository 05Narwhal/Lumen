import { useQuery } from "@tanstack/react-query";
import { useContextMenu } from "@/components/context-menu";
import { usePetBoxMenu } from "@/hooks/use-pet-box-menu";
import { usePets } from "@/hooks/use-pets";
import type { AssetPet } from "@/types/generated/AssetPet";
import { Link } from "react-router";
import { IconFocus2, IconMap, IconPhotoStar, IconUserScan } from "@tabler/icons-react";
import { usePeopleActions } from "@/hooks/use-people";
import { MiniMap } from "@/components/mini-map";
import { invoke } from "@/lib/ipc";
import type { AssetFace } from "@/types/generated/AssetFace";
import { StarRating } from "@/components/star-rating";
import { IconFolderOpen } from "@tabler/icons-react";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import { formatCaptured, formatDateTime } from "@/utils/date";
import { formatBytes } from "@/utils/format";
import { mediaUrl } from "@/utils/media";
import { TextSection } from "./text-section";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div className="info-panel__row">
      <span className="info-panel__label">{label}</span>
      <span className="info-panel__value">{value}</span>
    </div>
  );
}

const num = (v: unknown) => (typeof v === "number" ? v : undefined);
const str = (v: unknown) => (typeof v === "string" ? v : undefined);

/** `photos_media::hdr::HdrKind` codes, in words. */
const HDR_LABEL: Record<number, string> = {
  1: "HDR (gain map)",
  2: "HDR (PQ)",
  3: "HDR (HLG)",
};

/** The viewer's tagging state and actions (Round 4I), lent to the info panel. */
export interface FaceTools {
  /** Every box drawn over the photo (key `P`). */
  show: boolean;
  onToggleShow: () => void;
  /** Drawing a new box. */
  drawing: boolean;
  onAddFace: () => void;
  onTag: (face: AssetFace) => void;
  onUntag: (face: AssetFace) => void;
  /** The face whose row is under the pointer, outlined on the photo. */
  onHover: (face: number | null) => void;
}

/** One face: its crop, who it is, and what can be said about it. */
function FaceRow({ face, tools, onCover }: { face: AssetFace; tools?: FaceTools; onCover: (person: number, face: number) => void }) {
  const named = face.personId != null && face.name != null;
  return (
    <li className="info-panel__face" onMouseEnter={() => tools?.onHover(face.id)} onMouseLeave={() => tools?.onHover(null)}>
      <img className="info-panel__face-crop" src={mediaUrl("face", face.id)} alt="" />
      {named ? (
        <Link className="info-panel__face-name" to={`/people/${face.personId}`}>
          {face.name}
        </Link>
      ) : (
        <span className="info-panel__face-name" data-unknown>
          Unknown
        </span>
      )}
      {named && (
        <button
          type="button"
          className="info-panel__cover"
          aria-label={`Use as main photo of ${face.name}`}
          title={`Use as main photo of ${face.name}`}
          onClick={() => onCover(face.personId as number, face.id)}
        >
          <IconPhotoStar size={12} />
        </button>
      )}
      {tools &&
        (named ? (
          <button type="button" className="info-panel__face-action" onClick={() => tools.onUntag(face)}>
            Not {face.name}
          </button>
        ) : (
          <button type="button" className="info-panel__face-action" data-primary onClick={() => tools.onTag(face)}>
            Tag…
          </button>
        ))}
    </li>
  );
}

/** The viewer's *Show pets* state (Round 8B), lent to the info panel like `FaceTools`. */
export interface PetTools {
  /** Every pet's box drawn over the photo (Shift+P). */
  show: boolean;
  onToggleShow: () => void;
  /** The pet whose row is under the pointer, outlined on the photo. */
  onHover: (detection: number | null) => void;
}

/**
 * The pets in a photo (Round 4C): each named one links to its page; an unknown one can be
 * tagged as one of the pets of its species, with Undo. Each row has the pet box's menu
 * (Round 8B).
 */
function PetsInPhoto({ asset, tools }: { asset: number; tools?: PetTools }) {
  const here = useQuery({ queryKey: ["pets", "in", asset], queryFn: () => invoke<AssetPet[]>("asset_pets", { asset }) });
  const { pets } = usePets();
  const { menuFor, tagBox, picker } = usePetBoxMenu();
  const { open } = useContextMenu();
  if (!here.data?.length) return null;
  return (
    <div className="info-panel__group">
      <div className="info-panel__faces-head">
        <span className="info-panel__label">Pets</span>
        {tools && (
          <span className="info-panel__face-tools">
            <button type="button" className="info-panel__face-tool" aria-pressed={tools.show} title="Show pets (Shift+P)" onClick={tools.onToggleShow}>
              <IconFocus2 size={14} /> Show pets
            </button>
          </span>
        )}
      </div>
      <ul className="info-panel__faces">
        {here.data.map((d) => (
          <li
            key={d.id}
            className="info-panel__face"
            onMouseEnter={() => tools?.onHover(d.id)}
            onMouseLeave={() => tools?.onHover(null)}
            onContextMenu={(e) => open(e, menuFor(d, true))}
          >
            <img className="info-panel__face-crop" src={mediaUrl("pet", d.id)} alt="" />
            {d.petId != null ? (
              <Link className="info-panel__face-name" to={`/pets/${d.petId}`}>
                {d.name ?? `Unnamed ${d.species}`}
              </Link>
            ) : (
              <span className="info-panel__face-name" data-unknown>
                Unknown {d.species}
              </span>
            )}
            {d.petId == null && pets.some((p) => p.species === d.species) && (
              <button type="button" className="info-panel__face-action" data-primary onClick={() => tagBox(d)}>
                Tag…
              </button>
            )}
          </li>
        ))}
      </ul>
      {picker}
    </div>
  );
}

/**
 * The viewer's info column. `onOpen` shows another photo in the viewer (the one this was
 * exported from). With `faceTools` every face can be tagged, and new ones drawn; with
 * `petTools` the pets' boxes can be shown.
 */
/** The viewer's text (Round 13-3) for the info's *Text* section. */
export interface TextTools {
  lines: ShownTextLine[];
  reading: boolean;
  /** The overlay on the photo is on, and the way to toggle it (key T). */
  shown: boolean;
  onShowOnPhoto: () => void;
  onMenu: (text: string | null, e: React.MouseEvent) => void;
}

export function InfoPanel({
  asset,
  onReveal,
  onOpen,
  faceTools,
  petTools,
  text,
}: {
  asset: AssetDetail;
  onReveal: () => void;
  onOpen?: (id: number) => void;
  faceTools?: FaceTools;
  petTools?: PetTools;
  /** The text read in the photo (Round 13-3); absent while reading text is off. */
  text?: TextTools;
}) {
  const faces = useQuery({ queryKey: ["people", "faces", asset.id], queryFn: () => invoke<AssetFace[]>("faces_for_asset", { id: asset.id }) });
  const { setCover } = usePeopleActions();
  const m = asset.metadata ?? {};
  const exposure = [
    str(m.exposure) ? `${m.exposure} s` : null,
    num(m.aperture) ? `ƒ/${m.aperture}` : null,
    num(m.iso) ? `ISO ${m.iso}` : null,
    num(m.focal_length_mm) ? `${m.focal_length_mm} mm` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <aside className="info-panel" aria-label="Info">
      <h3 className="info-panel__title">{asset.filename}</h3>
      <div className="info-panel__rating">
        <StarRating value={asset.rating} readOnly />
        <span>{asset.rating ? `${asset.rating} of 5` : "Not rated"}</span>
      </div>
      <div className="info-panel__group">
        <Row label="Captured" value={asset.capturedAt ? formatCaptured(asset.capturedAt) : "Unknown"} />
        <Row label="Imported" value={formatDateTime(asset.importedAt)} />
        <Row label="Size" value={asset.width && asset.height ? `${asset.width} × ${asset.height} · ${formatBytes(asset.byteSize)}` : formatBytes(asset.byteSize)} />
        <Row label="Format" value={asset.extension.toUpperCase()} />
        {asset.hdr > 0 && <Row label="Dynamic range" value={HDR_LABEL[asset.hdr] ?? "HDR"} />}
        {asset.recoveredPct != null && <Row label="Recovered" value={`${asset.recoveredPct} % of a damaged file`} />}
        {asset.derivedFrom && (
          <Row
            label="Exported from"
            value={
              <button type="button" className="info-panel__link" onClick={() => onOpen?.(asset.derivedFrom!.id)}>
                {asset.derivedFrom.filename}
              </button>
            }
          />
        )}
      </div>
      {((faces.data?.length ?? 0) > 0 || faceTools) && (
        <div className="info-panel__group">
          <div className="info-panel__faces-head">
            <span className="info-panel__label">People</span>
            {faceTools && (
              <span className="info-panel__face-tools">
                <button
                  type="button"
                  className="info-panel__face-tool"
                  aria-pressed={faceTools.show}
                  title="Show faces (P)"
                  onClick={faceTools.onToggleShow}
                >
                  <IconFocus2 size={14} /> Show faces
                </button>
                <button type="button" className="info-panel__face-tool" aria-pressed={faceTools.drawing} title="Draw a box around someone the scan missed" onClick={faceTools.onAddFace}>
                  <IconUserScan size={14} /> Add a face
                </button>
              </span>
            )}
          </div>
          {(faces.data?.length ?? 0) > 0 ? (
            <ul className="info-panel__faces">
              {faces.data!.map((f) => (
                <FaceRow key={f.id} face={f} tools={faceTools} onCover={(person, face) => void setCover(person, face)} />
              ))}
            </ul>
          ) : (
            <p className="info-panel__faces-empty">{faceTools?.drawing ? "Drag a box around their face." : "No one found in this photo."}</p>
          )}
        </div>
      )}
      <PetsInPhoto asset={asset.id} tools={petTools} />
      {text && <TextSection key={asset.id} lines={text.lines} reading={text.reading} shown={text.shown} onShowOnPhoto={text.onShowOnPhoto} onMenu={text.onMenu} />}
      <div className="info-panel__group">
        <Row label="Camera" value={asset.deviceName} />
        <Row label="Lens" value={str(m.lens)} />
        <Row label="Exposure" value={exposure} />
      </div>
      {asset.gpsLat != null && asset.gpsLon != null && (
        <div className="info-panel__group">
          <MiniMap lat={asset.gpsLat} lon={asset.gpsLon} />
          {/* Only the names stored on the photo, never a live guess (the atlas called Idrija
              "Gorizia"); until one is stored, the coordinates. */}
          <Row
            label="Location"
            value={
              <>
                {asset.locality ?? asset.placeName ?? `${asset.gpsLat.toFixed(5)}, ${asset.gpsLon.toFixed(5)}`}
                {asset.placeSource === 1 && (asset.locality ?? asset.placeName) && (
                  <span className="info-panel__credit">© OpenStreetMap contributors</span>
                )}
              </>
            }
          />
          <Link className="info-panel__reveal" to="/map">
            <IconMap size={16} /> Open in Map
          </Link>
        </div>
      )}
      <div className="info-panel__group">
        <Row label="Path" value={<code className="info-panel__code">originals/{asset.relativePath}</code>} />
        <button type="button" className="info-panel__reveal" onClick={onReveal}>
          <IconFolderOpen size={16} /> Show file
        </button>
      </div>
    </aside>
  );
}
