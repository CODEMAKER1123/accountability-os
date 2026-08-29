// Always-on-top task and productivity widget (spec §25).

import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, type Commitment, type TodaySnapshot } from "@/lib/ipc";
import { fmtClockDuration, fmtDuration, fmtPct } from "@/lib/time";

function monitoringLabel(snapshot: TodaySnapshot): string {
  if (snapshot.monitoring_state === "paused") return "Monitoring paused";
  if (snapshot.monitoring_state === "permission_required") return "Monitoring needs permission";
  if (snapshot.work_hours.in_work_hours) {
    return snapshot.monitoring_state === "demo"
      ? "Demo recording work hours"
      : "Recording work hours";
  }
  return "Monitoring active · outside work hours";
}

function taskState(
  commitment: Commitment,
  activeId: number | null,
  pausedId: number | null,
): string {
  if (commitment.status === "completed") return "Done";
  if (commitment.status === "deferred") return "Deferred";
  if (commitment.status === "cancelled" || commitment.status === "dropped") return "Closed";
  if (commitment.id === activeId) return "Active";
  if (commitment.id === pausedId) return "Paused";
  return "Ready";
}

export default function WidgetWindow() {
  const [snap, setSnap] = useState<TodaySnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [, force] = useState(0);

  const load = useCallback(async () => {
    try {
      setSnap(await api.getTodaySnapshot());
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, []);

  const run = async (id: number | null, action: () => Promise<unknown>) => {
    if (busyId != null) return;
    setBusyId(id ?? -1);
    setError(null);
    try {
      await action();
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusyId(null);
    }
  };

  useEffect(() => {
    void load();
    const poll = setInterval(() => void load(), 5000);
    const tick = setInterval(() => force((n) => n + 1), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  }, [load]);

  const now = Math.floor(Date.now() / 1000);
  const active = snap?.active_commitment ?? null;
  const activeId = active?.id ?? null;
  const pausedId =
    activeId == null
      ? snap?.commitments.find((commitment) => commitment.status === "active")?.id ?? null
      : null;
  const elapsed = snap?.focus_session ? now - snap.focus_session.started_at : 0;
  const onBreak = Boolean(snap?.current_break && snap.current_break.ends_at > now);

  return (
    <div
      className="flex h-screen select-none flex-col border border-ink-600 bg-ink-950 p-3"
      data-tauri-drag-region
    >
      <div className="flex items-center justify-between" data-tauri-drag-region>
        <span className="text-2xs font-bold uppercase tracking-widest text-ink-400">
          Accountability OS
        </span>
        <button
          className="text-ink-600 hover:text-ink-300"
          title="Close widget"
          onClick={() => void run(null, () => api.setWidgetVisible(false))}
        >
          ✕
        </button>
      </div>

      {error && snap && (
        <p className="mt-1 line-clamp-2 text-2xs text-distracted" role="alert">
          {error}
        </p>
      )}

      {error && !snap ? (
        <div className="mt-2 flex flex-1 flex-col">
          <p className="text-xs font-medium text-distracted">Widget could not load</p>
          <p className="mt-1 line-clamp-3 text-2xs text-ink-400">{error}</p>
          <button className="btn mt-auto py-1 text-2xs" onClick={() => void load()}>
            Retry
          </button>
        </div>
      ) : !snap ? (
        <div className="mt-2 flex flex-1 items-center text-xs text-ink-400">Loading…</div>
      ) : (
        <>
          <div className="mt-2 grid grid-cols-[auto_1fr] items-center gap-3 rounded-md border border-ink-700 bg-ink-900 px-3 py-2">
            <div>
              <p className="font-mono text-2xl font-semibold tabular-nums text-ink-50">
                {fmtPct(snap.work_hours.productivity)}
              </p>
              <p className="text-2xs text-ink-500">productivity</p>
            </div>
            <div className="min-w-0 border-l border-ink-700 pl-3">
              <p
                className={`truncate text-xs font-medium ${
                  snap.monitoring_state === "paused" ||
                  snap.monitoring_state === "permission_required"
                    ? "text-warn"
                    : "text-focus"
                }`}
              >
                {monitoringLabel(snap)}
              </p>
              <p className="mt-0.5 truncate text-2xs text-ink-500">
                Tracked {fmtDuration(snap.work_hours.tracked_secs)} · Gap{" "}
                {fmtDuration(snap.work_hours.gap_secs)}
              </p>
            </div>
          </div>

          {onBreak && snap.current_break ? (
            <div className="mt-2 flex items-center justify-between rounded-md border border-accent/40 bg-accent/5 px-3 py-2">
              <div>
                <p className="text-2xs font-medium uppercase tracking-wider text-accent">Break</p>
                <p className="font-mono text-lg text-ink-50">
                  {fmtClockDuration(snap.current_break.ends_at - now)}
                </p>
              </div>
              <button
                className="btn py-1 text-2xs"
                disabled={busyId != null}
                onClick={() => void run(null, () => api.endBreakNow())}
              >
                End break
              </button>
            </div>
          ) : active ? (
            <div className="mt-2 flex items-center gap-2 rounded-md border border-focus/40 bg-focus/5 px-3 py-2">
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium text-ink-50" title={active.title}>
                  {active.title}
                </p>
                <p className="font-mono text-base tabular-nums text-focus">
                  {fmtClockDuration(elapsed)} active
                </p>
              </div>
              <button
                className="btn py-1 text-2xs"
                disabled={busyId != null}
                onClick={() => void run(active.id, () => api.completeCommitment(active.id))}
              >
                Done
              </button>
              <button
                className="btn py-1 text-2xs"
                disabled={busyId != null}
                onClick={() => void run(active.id, () => api.pauseFocus())}
              >
                Pause
              </button>
            </div>
          ) : (
            <p className="mt-2 rounded-md border border-ink-700 bg-ink-900 px-3 py-2 text-2xs text-ink-400">
              Focus is paused. Desktop activity monitoring continues independently.
            </p>
          )}

          <div className="mt-2 min-h-0 flex-1">
            <div className="mb-1 flex items-center justify-between">
              <p className="text-2xs font-semibold uppercase tracking-wider text-ink-500">
                Today's tasks
              </p>
              <span className="text-2xs text-ink-600">{snap.commitments.length}/3</span>
            </div>
            {snap.commitments.length === 0 ? (
              <p className="rounded-md border border-ink-700 px-3 py-2 text-xs text-ink-400">
                No plan yet. Open the app to plan your day.
              </p>
            ) : (
              <ol className="space-y-1">
                {snap.commitments.slice(0, 3).map((commitment) => {
                  const state = taskState(commitment, activeId, pausedId);
                  const terminal = ["completed", "deferred", "cancelled", "dropped"].includes(
                    commitment.status,
                  );
                  const canStart =
                    !onBreak &&
                    !terminal &&
                    activeId == null &&
                    (pausedId == null || pausedId === commitment.id);
                  return (
                    <li
                      key={commitment.id}
                      className="flex items-center gap-2 rounded-md border border-ink-700 bg-ink-900 px-2.5 py-1.5"
                    >
                      <span className="w-3 font-mono text-2xs text-ink-600">
                        {commitment.rank}.
                      </span>
                      <div className="min-w-0 flex-1">
                        <p
                          className={`truncate text-xs ${
                            commitment.status === "completed"
                              ? "text-ink-500 line-through"
                              : "text-ink-100"
                          }`}
                          title={commitment.title}
                        >
                          {commitment.title}
                        </p>
                        <p
                          className={`text-2xs ${
                            state === "Active"
                              ? "text-focus"
                              : state === "Paused"
                                ? "text-warn"
                                : "text-ink-500"
                          }`}
                        >
                          {state}
                          {commitment.steps.length > 0 &&
                            ` · ${commitment.steps.filter((step) => step.completed).length}/${commitment.steps.length} steps`}
                        </p>
                      </div>
                      {canStart && (
                        <button
                          className="btn py-1 text-2xs"
                          disabled={busyId != null}
                          onClick={() =>
                            void run(commitment.id, () => api.startCommitment(commitment.id))
                          }
                        >
                          {busyId === commitment.id
                            ? "Starting…"
                            : pausedId === commitment.id
                              ? "Resume"
                              : "Start"}
                        </button>
                      )}
                    </li>
                  );
                })}
              </ol>
            )}
          </div>

          <button
            className="btn mt-2 py-1 text-2xs"
            disabled={busyId != null}
            onClick={() => void run(null, () => api.showMainWindow())}
          >
            Open app
          </button>
        </>
      )}
    </div>
  );
}
