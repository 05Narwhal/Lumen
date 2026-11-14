import { IconCheck, IconX } from "@tabler/icons-react";
import { Menu } from "@mantine/core";
import { AnimatePresence, motion } from "motion/react";
import type { TagStore } from "@/hooks/use-tag-search";
import { leaves, type Path } from "@/lib/search-expr";
import { tagKey } from "@/lib/search-tags";
import type { Join } from "@/types/generated/Join";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagItem } from "@/types/generated/TagItem";
import type { TagSeq } from "@/types/generated/TagSeq";
import { TagIcon, TagWhere } from "./gui";

/** Tags arrive and leave on the same spring as the sidebar's pill. */
const SPRING = { type: "spring", stiffness: 520, damping: 38, mass: 0.8 } as const;

/** The edits a row can make; a row without them is read-only. */
export type RowOps = Pick<TagStore, "removeAt" | "setJoin" | "toggleNot" | "groupPair" | "ungroup">;

const CHOICES: { join: Join; not: boolean; label: string }[] = [
  { join: "and", not: false, label: "and" },
  { join: "or", not: false, label: "or" },
  { join: "and", not: true, label: "and not" },
  { join: "or", not: true, label: "or not" },
];

/** A stable key for an item, so a chip keeps its identity while the operators around it change. */
function itemKey(item: TagItem): string {
  return item.type === "tag" ? tagKey(item.tag) : `(${leaves(item.seq).map((l) => tagKey(l.tag)).join("|")})`;
}

const FADE = { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 } } as const;

/** One chip: icon, label, ×. Its label opens *Exclude / Include* and *Remove* unless the row is read-only. */
function Chip({ tag, excluded, path, ops }: { tag: SearchTag; excluded: boolean; path: Path; ops?: RowOps }) {
  const label = (
    <>
      {tag.value.type === "text" ? `“${tag.label}”` : tag.label}
      <TagWhere value={tag.value} />
    </>
  );
  return (
    <motion.span
      layout
      role="listitem"
      className="tag-bar__tag"
      data-type={tag.value.type}
      data-not={excluded || undefined}
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      transition={SPRING}
    >
      <TagIcon value={tag.value} size={13} />
      {ops ? (
        <Menu withinPortal position="bottom-start" shadow="md">
          <Menu.Target>
            <button type="button" className="tag-bar__label tag-bar__label--button" aria-label={`${tag.label} — change`}>
              {label}
            </button>
          </Menu.Target>
          <Menu.Dropdown>
            <Menu.Item onClick={() => ops.toggleNot(path)}>{excluded ? "Include" : "Exclude"}</Menu.Item>
            <Menu.Item onClick={() => ops.removeAt(path)}>Remove</Menu.Item>
          </Menu.Dropdown>
        </Menu>
      ) : (
        <span className="tag-bar__label">{label}</span>
      )}
      {ops && (
        <button type="button" className="tag-bar__remove" aria-label={`Remove ${tag.label}`} onClick={() => ops.removeAt(path)}>
          <IconX size={12} />
        </button>
      )}
    </motion.span>
  );
}

/** `and` / `or` / `and not` / `or not` between two items; a click opens the four choices and *Group these two* or *Ungroup*. */
function JoinButton({ join, not, group, index, ops }: { join: Join; not: boolean; group: Path; index: number; ops?: RowOps }) {
  const text = not ? `${join} not` : join;
  const strong = join === "or" || not;
  if (!ops) {
    return (
      <motion.span layout className="tag-bar__join" data-strong={strong} {...FADE}>
        {text}
      </motion.span>
    );
  }
  const inGroup = group.length > 0;
  return (
    <motion.span layout className="tag-bar__join-wrap" {...FADE}>
      <Menu withinPortal position="bottom-start" shadow="md">
        <Menu.Target>
          <button type="button" className="tag-bar__join" data-strong={strong} aria-label={`${text} — change how these combine`}>
            {text}
          </button>
        </Menu.Target>
        <Menu.Dropdown>
          {CHOICES.map((c) => (
            <Menu.Item
              key={c.label}
              rightSection={c.join === join && c.not === not ? <IconCheck size={14} /> : undefined}
              onClick={() => ops.setJoin(group, index, c.join, c.not)}
            >
              {c.label}
            </Menu.Item>
          ))}
          <Menu.Divider />
          {inGroup ? (
            <Menu.Item onClick={() => ops.ungroup(group)}>Ungroup</Menu.Item>
          ) : (
            // An `and` pair has no brackets to show: repair folds it back, so only `or` can be grouped.
            <Menu.Item disabled={join !== "or"} onClick={() => ops.groupPair(group, index)}>
              Group these two
            </Menu.Item>
          )}
        </Menu.Dropdown>
      </Menu>
    </motion.span>
  );
}

/** The lone `not` before a negated first item; a click includes it again. */
function LeadingNot({ path, ops }: { path: Path; ops?: RowOps }) {
  return ops ? (
    <motion.button layout type="button" className="tag-bar__join" data-strong aria-label="not — include these again" onClick={() => ops.toggleNot(path)} {...FADE}>
      not
    </motion.button>
  ) : (
    <motion.span layout className="tag-bar__join" data-strong {...FADE}>
      not
    </motion.span>
  );
}

/**
 * A sequence of search tags drawn with its operators (Round 20 §3): a join button before every
 * item but the first, groups as pills. Quiet `and`, stronger `or` and `not`, so `and` runs read
 * as clusters. With no `ops` the same row is drawn read-only (the field's *Read as* preview).
 */
export function TokenRow({ seq, ops, group = [] }: { seq: TagSeq; ops?: RowOps; group?: Path }) {
  return (
    <AnimatePresence initial={false}>
      {seq.items.flatMap((item, i) => {
        const path = [...group, i];
        const key = itemKey(item);
        const join = i > 0 ? (seq.joins[i - 1] ?? "and") : null;
        const lead =
          join !== null ? (
            <JoinButton key={`join:${key}`} join={join} not={!!item.not} group={group} index={i - 1} ops={ops} />
          ) : item.not ? (
            <LeadingNot key={`not:${key}`} path={path} ops={ops} />
          ) : null;
        const body =
          item.type === "tag" ? (
            <Chip key={key} tag={item.tag} excluded={!!item.not} path={path} ops={ops} />
          ) : (
            <motion.span key={key} layout role="group" className="tag-bar__group-pill" aria-label="Grouped tags" transition={SPRING} {...FADE}>
              <TokenRow seq={item.seq} ops={ops} group={path} />
            </motion.span>
          );
        return lead ? [lead, body] : [body];
      })}
    </AnimatePresence>
  );
}
