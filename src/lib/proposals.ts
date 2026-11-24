import type { ProposalValue } from "@/types/generated/ProposalValue";

/** What a plugin can propose (Round 25 P4b); the review route's `:kind`. */
export type ProposalKind = ProposalValue["kind"];

export const PROPOSAL_KINDS: ProposalKind[] = ["location", "date", "tags", "label", "person", "title", "caption"];

export const LABEL_COLOURS = ["Red", "Yellow", "Green", "Blue", "Purple"] as const;

export const isProposalKind = (s: string): s is ProposalKind => (PROPOSAL_KINDS as string[]).includes(s);

/** A date is wall-clock text; reading it as UTC and showing it as UTC keeps it as written. */
function dateText(wall: string): string {
  const d = new Date(`${wall}Z`);
  if (Number.isNaN(d.getTime())) return wall;
  return d.toLocaleString(undefined, { timeZone: "UTC", dateStyle: "medium", timeStyle: "short" });
}

/** A proposed or current value in words. */
export function describeValue(v: ProposalValue): string {
  switch (v.kind) {
    case "location":
      return `${v.value.lat.toFixed(4)}, ${v.value.lon.toFixed(4)}`;
    case "date":
      return dateText(v.value);
    case "tags":
      return v.value.join(", ");
    case "person":
      return v.value.name;
    default:
      return v.value;
  }
}

/** The text an edit box starts with. */
export function editText(v: ProposalValue): string {
  switch (v.kind) {
    case "location":
      return `${v.value.lat}, ${v.value.lon}`;
    case "tags":
      return v.value.join(", ");
    case "person":
      return v.value.name;
    default:
      return v.value;
  }
}

/**
 * What the person typed, as a value of `kind`, or `null` when it isn't one. A person's value
 * keeps the face of `like` (the proposal being edited): an edit names the same face.
 */
export function valueFromText(kind: ProposalKind, text: string, like?: ProposalValue): ProposalValue | null {
  const t = text.trim();
  if (!t) return null;
  switch (kind) {
    case "tags": {
      const tags: string[] = [];
      for (const part of t.split(",")) {
        const tag = part.trim();
        if (tag && !tags.some((x) => x.toLowerCase() === tag.toLowerCase())) tags.push(tag);
      }
      return tags.length ? { kind, value: tags } : null;
    }
    case "location": {
      const m = /^(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)$/.exec(t);
      if (!m) return null;
      const [lat, lon] = [Number(m[1]), Number(m[2])];
      return Math.abs(lat) <= 90 && Math.abs(lon) <= 180 ? { kind, value: { lat, lon } } : null;
    }
    case "date": {
      const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})(?::(\d{2}))?$/.exec(t);
      return m ? { kind, value: `${m[1]}T${m[2]}:${m[3] ?? "00"}` } : null;
    }
    case "person":
      return like?.kind === "person" ? { kind, value: { face: like.value.face, name: t } } : null;
    default:
      return { kind, value: t };
  }
}

/** Names the heading and the empty line of each kind's review. */
export const KIND_WORDS: Record<ProposalKind, { title: string; noun: string }> = {
  location: { title: "Review places", noun: "place" },
  date: { title: "Review dates", noun: "date" },
  tags: { title: "Review tags", noun: "tag" },
  label: { title: "Review labels", noun: "label" },
  person: { title: "Review names", noun: "name" },
  title: { title: "Review titles", noun: "title" },
  caption: { title: "Review captions", noun: "caption" },
};
