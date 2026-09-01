// Daily Plan — today's contract, or the door into the interview (spec §4).

import { PriorityTag } from "@/components/shared";
import { CommitmentSteps } from "@/components/CommitmentSteps";
import { useStore } from "@/lib/store";
import { fmtDuration, fmtTime } from "@/lib/time";

export default function Plan() {
  const { snapshot, snapshotLoading, setModal } = useStore();
  if (!snapshot) {
    return (
      <div className="mx-auto max-w-3xl p-6 text-xs text-ink-400" role="status">
        {snapshotLoading ? "Loading today’s plan…" : "Today’s plan is unavailable."}
      </div>
    );
  }
  const plan = snapshot?.plan ?? null;
  const commitments = snapshot?.commitments ?? [];
  const locked = plan?.locked_at != null;

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-6">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-ink-50">Daily Plan</h1>
        {!locked ? (
          <button className="btn btn-primary" onClick={() => setModal({ kind: "interview" })}>
            Start morning interview
          </button>
        ) : plan?.ended_at == null ? (
          <button className="btn" onClick={() => setModal({ kind: "interview", mode: "edit" })}>
            Edit today's plan
          </button>
        ) : null}
      </div>

      {plan?.is_day_off ? (
        <div className="card">
          <p className="text-sm text-ink-200">Today is marked as off. No accountability today.</p>
        </div>
      ) : !locked ? (
        <div className="card">
          <p className="text-sm text-ink-200">No plan locked yet.</p>
          <p className="mt-1 text-xs text-ink-400">
            The interview asks what must be true today, forces a ranking of at most three
            outcomes, and locks the contract.
          </p>
        </div>
      ) : (
        <>
          <div className="card">
            <div className="mb-3 flex items-center justify-between">
              <div>
                <p className="section-title">Today's contract</p>
                <p className="mt-1 text-2xs text-ink-500">
                  Locked protects your accountability record. Use Edit today's plan to revise it
                  without losing focus history or checked steps.
                </p>
              </div>
              <p className="shrink-0 text-2xs text-ink-500">
                {plan!.ended_at != null ? "read-only · ended " : "locked "}
                {fmtTime(plan!.ended_at ?? plan!.locked_at!)}
              </p>
            </div>
            <ol className="space-y-2">
              {commitments.map((c) => (
                <li
                  key={c.id}
                  className="flex items-start gap-3 rounded-md border border-ink-700 bg-ink-850 px-3 py-2"
                >
                  <span className="font-mono text-sm text-ink-500">{c.rank}.</span>
                  <div className="min-w-0 flex-1">
                    <p
                      className={`text-[13px] font-medium ${
                        c.status === "completed" ? "text-ink-500 line-through" : "text-ink-50"
                      }`}
                    >
                      {c.title}
                    </p>
                    {c.done_definition && (
                      <p className="text-2xs text-ink-400">DONE = {c.done_definition}</p>
                    )}
                    <p className="text-2xs text-ink-500">
                      {c.estimated_minutes != null && `~${fmtDuration(c.estimated_minutes * 60)} focused · `}
                      status: {c.status}
                      {c.outcome_reason && ` · ${c.outcome_reason.replaceAll("_", " ")}`}
                    </p>
                    <CommitmentSteps
                      commitment={c}
                      allowBreakdown={plan!.ended_at == null}
                    />
                  </div>
                  <PriorityTag priority={c.priority} />
                </li>
              ))}
            </ol>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="card">
              <p className="section-title mb-1">Likely distraction</p>
              <p className="text-[13px] text-ink-100">{plan!.likely_distraction || "—"}</p>
            </div>
            <div className="card">
              <p className="section-title mb-1">Countermeasure</p>
              <p className="text-[13px] text-ink-100">{plan!.countermeasure || "—"}</p>
            </div>
          </div>
          <p className="text-2xs text-ink-500">
            Most important work scheduled: {plan!.most_important_when.replace("specific:", "at ")}
          </p>
        </>
      )}
    </div>
  );
}
