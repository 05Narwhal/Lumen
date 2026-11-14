import { Button, Select, Switch } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef } from "react";
import { StatusBadge } from "@/components/status-badge";
import type { useSettings } from "@/hooks/use-settings";
import { experimentalCatalogQuery } from "@/lib/experimental";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AiTaskKind } from "@/types/generated/AiTaskKind";
import type { Experimental } from "@/types/generated/Experimental";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";
import type { ModelView } from "@/types/generated/ModelView";
import type { Settings } from "@/types/generated/Settings";
import type { TaskChoice } from "@/types/generated/TaskChoice";
import type { Tier } from "@/types/generated/Tier";
import type { TierView } from "@/types/generated/TierView";
import { SettingGroup, SettingRow } from "./row";

type Update = ReturnType<typeof useSettings>["update"];

const LABELS: Record<AiTaskKind, string> = {
  auto: "Auto correction (Ray)",
  retouch: "Retouch (Prism)",
  search: "Search (Spectrum)",
  text: "Text in photos (Glyph)",
  masks: "AI masks (Facet)",
};

/** The switch each switchable task keeps in `smart.experimental` (Round 12; reading text, Round 13; masks, 8f). */
const SWITCHES: Partial<Record<AiTaskKind, "autoModel" | "retouch" | "readText" | "aiMasks">> = { auto: "autoModel", retouch: "retouch", text: "readText", masks: "aiMasks" };

const TIER_NAMES: Record<Tier, string> = { low: "Low", high: "High", ultra: "Ultra" };

const OFF: Experimental = { autoModel: false, retouch: false, readText: false, aiMasks: false, tasks: { auto: { model: null, tier: "auto" }, retouch: { model: null, tier: "auto" } } };

/** A model time as the tab says it: "180 ms", "4.1 s". */
export function formatMs(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/** Where a timing was measured, as the tab says it. */
function where(ep: string | null | undefined): string {
  if (!ep) return "";
  return ep === "cpu" ? " on the processor" : " on the graphics card";
}

/** One Performance option: "High · 180 ms on the graphics card · ΔE 3.9"; an invalid or failed tier says so. */
export function tierLabel(t: TierView): string {
  if (!t.ready) return `${TIER_NAMES[t.tier]} · ${t.reason ?? "can't be used"}`;
  if (t.failed) return `${TIER_NAMES[t.tier]} · Could not run here`;
  return [TIER_NAMES[t.tier], t.ms != null ? `${formatMs(t.ms)}${where(t.ep)}` : null, t.valDe != null ? `ΔE ${t.valDe.toFixed(1)}` : null]
    .filter(Boolean)
    .join(" · ");
}

/** "Ultra could not run here: …" for each tier of `model` that failed its timing. */
export function failedNotes(model: ModelView | undefined): string[] {
  return (model?.tiers ?? []).filter((t) => t.ready && t.failed).map((t) => `${TIER_NAMES[t.tier]} could not run here: ${t.failed}`);
}

/** How long the tab keeps asking while a PC test runs; after that the test runs on unwatched. */
export const TESTING_POLL_MS = 60_000;

/** The line under a task: what runs, or why nothing can. */
export function experimentalLine(row: ExperimentalTask): string {
  if (row.builtin) return `Ready · ${row.builtin}`;
  // Glyph runs the stock text models from the dev folder until it ships (Round 13).
  if (row.task === "text" && row.models.length === 0) return "Needs the text models";
  // A download that failed says why (Round 18 C1).
  if (row.problem) return row.problem;
  // Facet (8f) is fetched only once its switch is on.
  if (row.task === "masks" && row.models.length === 0) return "Downloads Lumen Facet (48 MB) once on";
  const model = row.models.find((m) => m.id === row.model) ?? row.models[0];
  if (!model) return row.task === "retouch" ? "Retouch arrives with Lumen Prism." : "No trained model installed";
  const tier = model.tiers.find((t) => t.tier === row.tier) ?? model.tiers.find((t) => t.ready);
  if (!tier?.ready) return `Can't use the model: ${model.tiers.find((t) => !t.ready)?.reason ?? "unknown reason"}`;
  return ["Ready", model.name, tier.valDe != null ? `ΔE ${tier.valDe.toFixed(1)}` : null].filter(Boolean).join(" · ");
}

/** "Ultra takes about 4.1 s here; Lumen recommends High." when a chosen tier is over budget. */
export function budgetWarning(row: ExperimentalTask, choice: TaskChoice | undefined, model: ModelView | undefined): string | null {
  if (!choice || choice.tier === "auto" || row.budgetMs == null) return null;
  const t = model?.tiers.find((x) => x.tier === choice.tier);
  if (!t?.ready || t.ms == null || t.ms <= row.budgetMs) return null;
  return `${TIER_NAMES[t.tier]} takes about ${formatMs(t.ms)} here; Lumen recommends ${TIER_NAMES[row.recommended]}.`;
}

/** Settings → Experimental (Round 14): a model and a performance tier per AI task. */
export function ExperimentalSection({ settings, update }: { settings: Settings; update: Update }) {
  const client = useQueryClient();
  // When the tab first saw a test running: polling stops after TESTING_POLL_MS, so a test that
  // never ends can never keep the tab refetching.
  const testingSince = useRef<number | null>(null);
  const catalog = useQuery({
    ...experimentalCatalogQuery,
    refetchInterval: (q) => {
      if (!q.state.data?.some((r) => r.testing)) {
        testingSince.current = null;
        return false;
      }
      testingSince.current ??= Date.now();
      return Date.now() - testingSince.current < TESTING_POLL_MS ? 2000 : false;
    },
  });
  const smart = settings.smart.enabled ?? true;
  const ex = { ...OFF, ...settings.smart.experimental, tasks: { ...OFF.tasks, ...settings.smart.experimental?.tasks } };

  const save = (next: Partial<Experimental>) =>
    update((s) => {
      const cur = { ...OFF, ...s.smart.experimental, tasks: { ...OFF.tasks, ...s.smart.experimental?.tasks } };
      return { ...s, smart: { ...s.smart, experimental: { ...cur, ...next } } };
    });
  const choose = (task: "auto" | "retouch", choice: Partial<TaskChoice>) => save({ tasks: { ...ex.tasks, [task]: { ...ex.tasks[task], ...choice } } });

  const retest = async () => {
    testingSince.current = null;
    await invoke("retest_models");
    await client.invalidateQueries({ queryKey: queryKeys.experimentalCatalog });
  };

  return (
    <SettingGroup>
      {(catalog.data ?? []).map((row) => {
        const label = LABELS[row.task];
        const key = SWITCHES[row.task];
        const choice = row.task === "auto" || row.task === "retouch" ? ex.tasks[row.task] : undefined;
        const model = row.models.find((m) => m.id === row.model) ?? row.models[0];
        const empty = row.models.length === 0;
        const developing = row.task === "retouch" && empty;
        const needsModels = row.task === "text" && !row.builtin;
        const warning = budgetWarning(row, choice, model);
        const failures = row.builtin ? [] : failedNotes(model);
        const tierValue = choice && choice.tier !== "auto" && model?.tiers.some((t) => t.tier === choice.tier && t.ready) ? choice.tier : "auto";
        return (
          <div className="settings-task" key={row.task}>
            <SettingRow
              label={
                <span className="settings__inline">
                  {label}
                  {developing && <StatusBadge>In development</StatusBadge>}
                </span>
              }
              hint={experimentalLine(row)}
            >
              {key && (
                <Switch
                  aria-label={label}
                  checked={ex[key]}
                  disabled={!smart || developing || needsModels}
                  onChange={(e) => save({ [key]: e.currentTarget.checked })}
                />
              )}
            </SettingRow>
            <div className="settings-task__picks">
              <Select
                label="Model"
                aria-label={`${label} model`}
                disabled={!smart || empty || !row.switchable}
                value={row.builtin ? "builtin" : (model?.id ?? "")}
                data={
                  row.builtin
                    ? [{ value: "builtin", label: row.builtin }]
                    : empty
                      ? [{ value: "", label: "No model yet" }]
                      : row.models.map((m) => ({ value: m.id, label: m.name }))
                }
                onChange={(v) => v !== null && choice && choose(row.task as "auto" | "retouch", { model: v })}
              />
              <Select
                label="Performance"
                aria-label={`${label} performance`}
                disabled={!smart || empty || !row.switchable}
                value={row.builtin ? "one" : empty ? "" : tierValue}
                data={
                  row.builtin || row.task === "search"
                    ? [{ value: "one", label: "One size" }]
                    : empty
                      ? [{ value: "", label: "No model yet" }]
                      : [
                        { value: "auto", label: `Automatic (${TIER_NAMES[row.recommended]})` },
                        ...(model?.tiers ?? []).map((t) => ({ value: t.tier, label: tierLabel(t), disabled: !t.ready || !!t.failed })),
                      ]
                }
                onChange={(v) => v !== null && choice && choose(row.task as "auto" | "retouch", { tier: v as TaskChoice["tier"] })}
              />
            </div>
            {(warning || row.testing) && (
              <p className="settings__note settings-task__status" role="status">
                {warning ?? "Testing this PC…"}
              </p>
            )}
            {failures.map((f) => (
              <p className="settings__note settings-task__status" key={f}>
                {f}
              </p>
            ))}
          </div>
        );
      })}
      {catalog.data && (
        <SettingRow hint="Lumen times each model on this computer and picks the best tier that still keeps edits quick.">
          <Button variant="default" size="xs" onClick={() => void retest()}>
            Re-test
          </Button>
        </SettingRow>
      )}
    </SettingGroup>
  );
}
