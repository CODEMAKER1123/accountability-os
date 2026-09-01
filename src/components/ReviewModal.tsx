// End-of-day review (spec §21–22): the day's numbers, per-commitment
// outcomes with miss reasons, optional AI analysis.

import { useCallback, useEffect, useState } from "react";

import Dialog from "@/components/Dialog";
import { ErrorBanner, ScoreRing } from "@/components/shared";
import { api, errorMessage, type ReviewData } from "@/lib/ipc";
import { useStore } from "@/lib/store";
import { fmtDuration, fmtPct } from "@/lib/time";

const MISS_REASONS: [string, string][] = [
  ["underestimated_time", "Underestimated time"],
  ["priorities_changed", "Priorities changed"],
  ["distraction", "Distraction"],
  ["blocked", "Blocked"],
  ["too_many_commitments", "Too many commitments"],
  ["task_unclear", "Task was unclear"],
  ["unexpected_work", "Unexpected work"],
  ["other", "Other"],
];

export default function ReviewModal() {
  const { setModal, refreshSnapshot, settings } = useStore();
  const [data, setData] = useState<ReviewData | null>(null);
  const [items, setItems] = useState<
    Record<number, { completed: boolean; reason: string | null; note: string }>
  >({});
  const [aiText, setAiText] = useState<string | null>(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState(false);
  const [loading, setLoading] = useState(true);

  const loadReview = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const d = await api.getReviewData();
      setData(d);
      setAiText(d.ai_summary);
      setSubmitted(d.already_reviewed);
      const init: typeof items = {};
      for (const c of d.commitments) {
        init[c.id] = {
          completed: c.status === "completed",
          reason: MISS_REASONS.some(([id]) => id === c.outcome_reason) ? c.outcome_reason : null,
          note: c.outcome_note ?? "",
        };
      }
      setItems(init);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadReview();
  }, [loadReview]);

  const delay = async () => {
    await api.delayReview(30);
    setModal(null);
  };

  const submit = async () => {
    if (!data) return;
    try {
      await api.submitReview({
        items: data.commitments.map((c) => ({
          commitment_id: c.id,
          completed: items[c.id]?.completed ?? false,
          reason: items[c.id]?.completed ? null : (items[c.id]?.reason ?? null),
          note: items[c.id]?.note || null,
        })),
      });
      setSubmitted(true);
      await refreshSnapshot();
      // Refresh the score panel post-submit.
      const d = await api.getReviewData();
      setData(d);
      if (settings?.ai_coaching_enabled) {
        setAiBusy(true);
        try {
          setAiText(await api.generateDailyAiReview());
        } catch {
          // AI summary is optional; the review stands without it.
        } finally {
          setAiBusy(false);
        }
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const canSubmit =
    data?.commitments.every((commitment) => {
      const item = items[commitment.id];
      return (
        item?.completed ||
        item?.reason != null ||
        commitment.status === "deferred" ||
        commitment.status === "cancelled" ||
        commitment.status === "dropped"
      );
    }) ?? false;
  const missingReasons = data
    ? data.commitments.filter((commitment) => {
        const item = items[commitment.id];
        return (
          !item?.completed &&
          !item?.reason &&
          !["deferred", "cancelled", "dropped"].includes(commitment.status)
        );
      }).length
    : 0;

  return (
    <Dialog labelledBy="daily-review-title" onClose={() => setModal(null)} panelClassName="flex max-h-full w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-ink-600 bg-ink-900 shadow-2xl">
        <div className="flex items-center justify-between border-b border-ink-700 px-5 py-3">
          <p id="daily-review-title" className="text-sm font-semibold text-ink-50">Daily Review</p>
          {!submitted && (
            <div className="flex gap-2">
              <button className="btn py-1 text-xs" onClick={() => void delay()}>
                Delay 30 minutes
              </button>
              <button className="btn py-1 text-xs" onClick={() => setModal(null)}>
                Continue working
              </button>
            </div>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {error && <ErrorBanner message={error} onRetry={() => void loadReview()} onDismiss={() => setError(null)} />}
          {loading ? (
            <p className="text-xs text-ink-400" role="status">Loading review…</p>
          ) : !data ? (
            <p className="text-xs text-ink-400">No locked plan today — nothing to review.</p>
          ) : (
            <div className="space-y-4">
              {/* The day in numbers */}
              <div className="flex items-center gap-6">
                <ScoreRing value={data.score.total} size={88} label="Execution score" />
                <dl className="grid flex-1 grid-cols-3 gap-x-4 gap-y-1 text-xs">
                  <Num k="Committed" v={String(data.commitments.length)} />
                  <Num
                    k="Completed"
                    v={String(Object.values(items).filter((i) => i.completed).length)}
                  />
                  <Num k="Alignment" v={fmtPct(data.score.alignment)} />
                  <Num k="Focus time" v={fmtDuration(data.score.focused_secs)} />
                  <Num k="Supporting" v={fmtDuration(data.score.supporting_secs)} />
                  <Num k="Neutral/admin" v={fmtDuration(data.score.neutral_secs)} />
                  <Num k="Distracted" v={fmtDuration(data.score.distracted_secs)} />
                  <Num k="Idle" v={fmtDuration(data.score.idle_secs)} />
                  <Num k="Switches" v={String(data.score.context_switches)} />
                </dl>
              </div>

              {/* Per-commitment outcomes */}
              <div className="space-y-2">
                {data.commitments.map((c) => {
                  const item = items[c.id] ?? { completed: false, reason: null, note: "" };
                  const focused =
                    data.commitment_progress.find((p) => p.commitment_id === c.id)?.focused_secs ??
                    0;
                  return (
                    <div key={c.id} className="rounded-md border border-ink-700 bg-ink-850 p-3">
                      <label className="flex cursor-pointer items-center gap-3">
                        <input
                          type="checkbox"
                          className="h-4 w-4 accent-[#4ea87c]"
                          checked={item.completed}
                          disabled={submitted}
                          onChange={(e) =>
                            setItems({ ...items, [c.id]: { ...item, completed: e.target.checked } })
                          }
                        />
                        <span className="flex-1 text-[13px] font-medium text-ink-50">{c.title}</span>
                        <span className="text-2xs text-ink-500">
                          {fmtDuration(focused)} focused
                          {c.estimated_minutes ? ` / ~${c.estimated_minutes}m est` : ""}
                        </span>
                      </label>
                      {!item.completed && !submitted && (
                        <div className="mt-2 space-y-2 pl-7">
                          <p className="text-2xs text-ink-400">Why wasn't this completed?</p>
                          <div className="flex flex-wrap gap-1">
                            {MISS_REASONS.map(([id, label]) => (
                              <button
                                key={id}
                                className={`rounded-full border px-2 py-0.5 text-2xs ${
                                  item.reason === id
                                    ? "border-accent bg-accent/10 text-ink-50"
                                    : "border-ink-600 text-ink-400 hover:border-ink-500"
                                }`}
                                onClick={() =>
                                  setItems({ ...items, [c.id]: { ...item, reason: id ?? null } })
                                }
                              >
                                {label}
                              </button>
                            ))}
                          </div>
                          <input
                            className="input"
                            placeholder="Explanation (optional)"
                            value={item.note}
                            onChange={(e) =>
                              setItems({ ...items, [c.id]: { ...item, note: e.target.value } })
                            }
                          />
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>

              {/* AI analysis */}
              {(aiText || aiBusy) && (
                <div className="rounded-md border border-ink-700 bg-ink-850 p-3">
                  <p className="section-title mb-1">Analysis</p>
                  {aiBusy ? (
                    <p className="text-xs text-ink-400">Analyzing the day…</p>
                  ) : (
                    <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-ink-100">
                      {aiText}
                    </p>
                  )}
                </div>
              )}

              <div className="flex justify-end gap-2">
                {submitted ? (
                  <button className="btn btn-primary" onClick={() => setModal(null)}>
                    Done
                  </button>
                ) : (
                  <div className="flex flex-col items-end gap-1">
                    <button className="btn btn-primary" onClick={() => void submit()} disabled={!canSubmit}>
                      Close out the day
                    </button>
                    {!canSubmit && missingReasons > 0 && (
                      <p className="text-2xs text-ink-500">
                        Mark each unfinished outcome completed or choose a miss reason ({missingReasons} remaining).
                      </p>
                    )}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
    </Dialog>
  );
}

function Num({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-ink-800 py-0.5">
      <dt className="text-ink-400">{k}</dt>
      <dd className="font-mono text-ink-100">{v}</dd>
    </div>
  );
}
