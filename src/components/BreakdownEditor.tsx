import { useState } from "react";

import { errorMessage, api, type BreakdownDetail } from "@/lib/ipc";
import {
  MAX_ACTION_STEPS,
  newStepSuggestions,
  normalizeStepTitles,
} from "@/lib/taskBreakdown";
import { useStore } from "@/lib/store";

const DETAIL_OPTIONS: { id: BreakdownDetail; label: string; count: string }[] = [
  { id: "simple", label: "Quick", count: "3–4" },
  { id: "standard", label: "Standard", count: "5–7" },
  { id: "detailed", label: "Detailed", count: "8–10" },
];

export function BreakdownEditor({
  goal,
  existingSteps,
  onSave,
  onClose,
}: {
  goal: string;
  existingSteps: string[];
  onSave: (steps: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const { settings, setView } = useStore();
  const [detail, setDetail] = useState<BreakdownDetail>("standard");
  const [draft, setDraft] = useState<string[]>([]);
  const [busy, setBusy] = useState<"generate" | "save" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const normalizedExisting = normalizeStepTitles(existingSteps);
  const capacity = Math.max(0, MAX_ACTION_STEPS - normalizedExisting.length);
  const ready = normalizeStepTitles(draft).slice(0, capacity);
  const aiDisabled = settings?.ai_coaching_enabled === false;

  const generate = async () => {
    if (busy || capacity === 0 || aiDisabled) return;
    setBusy("generate");
    setError(null);
    try {
      const result = await api.breakDownGoal(goal, detail);
      const suggestions = newStepSuggestions(result.steps, normalizedExisting);
      if (suggestions.length === 0) {
        setError("AI did not return any new steps. Add one manually or try another detail level.");
      } else {
        setDraft(suggestions);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section
      className="mt-3 space-y-3 rounded-md border border-accent/35 bg-ink-900 p-3"
      aria-label={`Break ${goal} into steps`}
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold text-ink-100">Break into steps</p>
          <p className="mt-0.5 text-2xs text-ink-400">
            Generate editable suggestions or add your own. Saved steps become checkable work.
          </p>
        </div>
        <button className="btn btn-ghost px-2 py-1" onClick={onClose} aria-label="Close step editor">
          Close
        </button>
      </div>

      {normalizedExisting.length > 0 && (
        <p className="rounded bg-ink-850 px-2 py-1.5 text-2xs text-ink-400">
          {normalizedExisting.length} step{normalizedExisting.length === 1 ? " is" : "s are"} already
          saved. You can add {capacity} more.
        </p>
      )}

      {capacity > 0 ? (
        <>
          <div className="flex flex-wrap items-end gap-2">
            <label className="space-y-1">
              <span className="block text-2xs text-ink-400">Breakdown size</span>
              <select
                className="input w-36"
                aria-label="Breakdown size"
                value={detail}
                onChange={(event) => setDetail(event.target.value as BreakdownDetail)}
                disabled={busy != null}
              >
                {DETAIL_OPTIONS.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label} ({option.count})
                  </option>
                ))}
              </select>
            </label>
            <button
              className="btn btn-primary"
              onClick={() => void generate()}
              disabled={busy != null || aiDisabled}
            >
              {busy === "generate" ? "Generating…" : draft.length > 0 ? "Regenerate with AI" : "Generate with AI"}
            </button>
            <button
              className="btn"
              onClick={() => {
                setError(null);
                setDraft((current) => (current.length < capacity ? [...current, ""] : current));
              }}
              disabled={busy != null || draft.length >= capacity}
            >
              Add a step manually
            </button>
          </div>

          {aiDisabled && (
            <p className="text-2xs text-ink-400">
              AI coaching is off. Add steps manually or{" "}
              <button
                className="text-accent underline underline-offset-2"
                onClick={() => {
                  onClose();
                  setView("settings");
                }}
              >
                enable AI in Settings
              </button>
              .
            </p>
          )}

          {draft.length > 0 && (
            <ol className="space-y-2">
              {draft.map((step, index) => (
                <li key={index} className="flex items-center gap-2">
                  <span className="w-5 text-right font-mono text-2xs text-ink-500">{index + 1}.</span>
                  <input
                    className="input flex-1"
                    aria-label={`Action step ${index + 1}`}
                    maxLength={300}
                    value={step}
                    placeholder="Describe one concrete action…"
                    onChange={(event) =>
                      setDraft((current) =>
                        current.map((title, stepIndex) =>
                          stepIndex === index ? event.target.value : title,
                        ),
                      )
                    }
                  />
                  <button
                    className="btn btn-ghost px-2 py-1 text-ink-500 hover:text-distracted"
                    aria-label={`Remove action step ${index + 1}`}
                    onClick={() =>
                      setDraft((current) => current.filter((_, stepIndex) => stepIndex !== index))
                    }
                    disabled={busy != null}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ol>
          )}

          {error && <p className="text-2xs text-distracted">{error}</p>}

          <div className="flex justify-end gap-2 border-t border-ink-700 pt-3">
            <button className="btn" onClick={onClose} disabled={busy != null}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={busy != null || ready.length === 0}
              onClick={async () => {
                setBusy("save");
                setError(null);
                try {
                  await onSave(ready);
                } catch (caught) {
                  setError(errorMessage(caught));
                  setBusy(null);
                }
              }}
            >
              {busy === "save" ? "Saving…" : `Save ${ready.length || ""} step${ready.length === 1 ? "" : "s"}`}
            </button>
          </div>
        </>
      ) : (
        <div className="flex items-center justify-between gap-3 rounded bg-ink-850 px-3 py-2">
          <p className="text-xs text-ink-300">This item already has the maximum of 12 steps.</p>
          <button className="btn" onClick={onClose}>Done</button>
        </div>
      )}
    </section>
  );
}
