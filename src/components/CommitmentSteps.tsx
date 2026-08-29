import { useState } from "react";

import { api, errorMessage, type Commitment } from "@/lib/ipc";
import { useStore } from "@/lib/store";

export function CommitmentSteps({ commitment }: { commitment: Commitment }) {
  const { refreshSnapshot } = useStore();
  const [busyIndex, setBusyIndex] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const terminal = ["completed", "deferred", "dropped", "cancelled"].includes(
    commitment.status,
  );

  if (commitment.steps.length === 0) return null;

  const completed = commitment.steps.filter((step) => step.completed).length;
  return (
    <div className="mt-3 rounded-md border border-ink-700 bg-ink-850 p-3">
      <div className="mb-2 flex items-center justify-between">
        <p className="section-title">Action steps</p>
        <p className="text-2xs text-ink-500">
          {completed}/{commitment.steps.length} complete
        </p>
      </div>
      <ol className="space-y-1.5">
        {commitment.steps.map((step, index) => (
          <li key={`${commitment.id}-step-${index}`}>
            <label className="flex cursor-pointer items-start gap-2 text-xs text-ink-200">
              <input
                aria-label={`${step.completed ? "Reopen" : "Complete"} step: ${step.title}`}
                type="checkbox"
                className="mt-0.5 h-3.5 w-3.5 accent-[#5b8def]"
                checked={step.completed}
                disabled={terminal || busyIndex != null}
                onChange={async (event) => {
                  setBusyIndex(index);
                  setError(null);
                  try {
                    await api.setCommitmentStepCompleted(
                      commitment.id,
                      index,
                      event.target.checked,
                    );
                    await refreshSnapshot();
                  } catch (caught) {
                    setError(errorMessage(caught));
                  } finally {
                    setBusyIndex(null);
                  }
                }}
              />
              <span className={step.completed ? "text-ink-500 line-through" : undefined}>
                {step.title}
              </span>
            </label>
          </li>
        ))}
      </ol>
      {error && <p className="mt-2 text-2xs text-distracted">{error}</p>}
    </div>
  );
}
