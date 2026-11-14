/** How long an AI action may run before its button shows anything (`--ai-delay`). */
export const AI_DELAY_MS = 250;

/** What a button says for each stage key the backend sends (`ai_task::AiTask::stage`). */
export function stageLabel(stage: string, done: number, total: number): string {
  switch (stage) {
    case "reading":
      return "Reading photo…";
    case "trying":
      return `Trying looks ${done}/${total}`;
    case "predicting":
      return "Using the model…";
    case "detecting":
      // Phase 8f: Lumen Facet looking for sky, subject and people.
      return "Finding masks…";
    case "applying":
      return "Applying…";
    default:
      return "Working…";
  }
}
