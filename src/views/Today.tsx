// Today — the command center (spec §4). Within 2 seconds it must answer:
// what should I be working on, am I on task, how is today going (spec §38).

import { useEffect, useState } from "react";

import { ClassBadge, ErrorBanner, PriorityTag, ProgressBar, ScoreRing } from "@/components/shared";
import { CommitmentSteps } from "@/components/CommitmentSteps";
import { api, errorMessage, type Commitment } from "@/lib/ipc";
import { useStore } from "@/lib/store";
import { fmtClockDuration, fmtDuration, fmtMinOfDay, fmtTime } from "@/lib/time";

export default function Today() {
  const { snapshot, refreshSnapshot, setModal } = useStore();
  const [, forceTick] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);

  const runAction = async (action: () => Promise<void>) => {
    setActionError(null);
    try {
      await action();
    } catch (error) {
      setActionError(errorMessage(error));
    }
  };

  // Local 1s ticker so timers run smoothly between 5s snapshot refreshes.
  useEffect(() => {
    const t = setInterval(() => forceTick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, []);

  if (!snapshot) {
    return <div className="p-6 text-ink-400">Loading…</div>;
  }

  const now = Math.floor(Date.now() / 1000);
  const active = snapshot.active_commitment;
  const activeCommitment = snapshot.commitments.find((c) => c.id === active?.id);
  const focusElapsed = snapshot.focus_session ? now - snapshot.focus_session.started_at : 0;
  const onBreak = snapshot.current_break && snapshot.current_break.ends_at > now;
  const planLocked = snapshot.plan?.locked_at != null;
  const dayEnded = snapshot.plan?.ended_at != null;
  const pending = snapshot.commitments.filter(
    (c) => c.status !== "completed" && c.status !== "cancelled" && c.status !== "deferred",
  );

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-6">
      {/* Header row */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-ink-50">Today</h1>
          <p className="text-xs text-ink-400">
            {new Date().toLocaleDateString([], {
              weekday: "long",
              month: "long",
              day: "numeric",
            })}
            {planLocked && !dayEnded && " · day locked"}
            {dayEnded && " · day ended"}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {!onBreak && (
            <button className="btn" onClick={() => setModal({ kind: "break" })}>
              Take break
            </button>
          )}
          {planLocked && !dayEnded && (
            <>
              <button
                className="btn"
                onClick={() => setModal({ kind: "interview", mode: "edit" })}
              >
                Edit plan
              </button>
              <button className="btn" onClick={() => setModal({ kind: "review" })}>
                End day
              </button>
            </>
          )}
        </div>
      </div>

      {actionError && (
        <ErrorBanner message={actionError} onDismiss={() => setActionError(null)} />
      )}

      {/* Break banner */}
      {onBreak && snapshot.current_break && (
        <div className="card flex items-center justify-between border-accent/40">
          <div>
            <p className="section-title text-accent">Planned break</p>
            <p className="mt-1 font-mono text-xl text-ink-50">
              {fmtClockDuration(snapshot.current_break.ends_at - now)}
            </p>
            <p className="text-xs text-ink-400">remaining — not counted as distraction</p>
          </div>
          <button
            className="btn"
            onClick={() =>
              void runAction(async () => {
                await api.endBreakNow();
                await refreshSnapshot();
              })
            }
          >
            End break now
          </button>
        </div>
      )}

      {/* Active commitment (spec §7) */}
      {active && activeCommitment ? (
        <ActiveCommitmentCard
          commitment={activeCommitment}
          focusElapsed={focusElapsed}
          focusedSecs={
            snapshot.commitment_progress.find((p) => p.commitment_id === active.id)?.focused_secs ??
            0
          }
          onError={setActionError}
        />
      ) : !planLocked ? (
        <div className="card flex items-center justify-between">
          <div>
            <p className="font-medium text-ink-100">No plan for today.</p>
            <p className="text-xs text-ink-400">
              Run the morning interview and commit to 1–3 outcomes.
            </p>
          </div>
          <button className="btn btn-primary" onClick={() => setModal({ kind: "interview" })}>
            Plan my day
          </button>
        </div>
      ) : null}

      {/* Commitments list */}
      {planLocked && snapshot.commitments.length > 0 && (
        <div className="card">
          <p className="section-title">Today's commitments</p>
          {!active && !dayEnded && pending.length > 0 && (
            <p className="mb-3 mt-1 text-xs text-ink-400">
              Focus is paused. Activity monitoring still records your workday; choose a
              commitment to resume focused tracking.
            </p>
          )}
          <div className="space-y-2">
            {snapshot.commitments.map((c) => (
              <CommitmentRow
                key={c.id}
                commitment={c}
                activeId={active?.id ?? null}
                onError={setActionError}
              />
            ))}
          </div>
        </div>
      )}

      {/* Live status + day totals */}
      <div className="grid grid-cols-3 gap-4">
        <div className="card col-span-2">
          <p className="section-title mb-3">Right now</p>
          {snapshot.current ? (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <div className="min-w-0">
                  <p className="truncate font-medium text-ink-100">
                    {snapshot.current.is_idle ? "Idle" : snapshot.current.app_name}
                    {snapshot.current.browser_domain && (
                      <span className="ml-2 text-xs text-ink-400">
                        {snapshot.current.browser_domain}
                      </span>
                    )}
                  </p>
                  <p className="truncate text-xs text-ink-400">
                    {snapshot.current.window_title || "—"}
                  </p>
                </div>
                <ClassBadge classification={snapshot.current.outcome.classification} />
              </div>
              <p className="text-2xs text-ink-500">
                since {fmtTime(snapshot.current.since)} · {snapshot.current.outcome.reason}
              </p>
              {snapshot.warned && (
                <p className="text-xs font-medium text-warn">
                  Off task for {fmtDuration(snapshot.distracted_secs)} — heading toward an
                  intervention.
                </p>
              )}
            </div>
          ) : (
            <p className="text-xs text-ink-500">
              {snapshot.monitoring_message ?? "Monitoring is not reporting activity."}
            </p>
          )}
          <div className="mt-4 grid grid-cols-5 gap-2 border-t border-ink-700 pt-3">
            <Stat label="Focused" value={fmtDuration(snapshot.totals.focused_secs)} cls="text-focus" />
            <Stat
              label="Supporting"
              value={fmtDuration(snapshot.totals.supporting_secs)}
              cls="text-supporting"
            />
            <Stat label="Neutral" value={fmtDuration(snapshot.totals.neutral_secs)} cls="text-neutralcat" />
            <Stat
              label="Distracted"
              value={fmtDuration(snapshot.totals.distracted_secs)}
              cls="text-distracted"
            />
            <Stat label="Idle" value={fmtDuration(snapshot.totals.idle_secs)} cls="text-idlecat" />
          </div>
          <p className="mt-3 text-2xs text-ink-500">
            Next check-in {fmtTime(snapshot.next_checkin_at)}
            {snapshot.extension_connected && " · browser extension connected"}
          </p>
          <p className="mt-1 text-2xs text-ink-500">
            Work hours {fmtMinOfDay(snapshot.work_hours.start_min)}–
            {fmtMinOfDay(snapshot.work_hours.end_min)} · tracked{" "}
            {fmtDuration(snapshot.work_hours.tracked_secs)} · untracked gap{" "}
            {fmtDuration(snapshot.work_hours.gap_secs)}. Monitoring continues when a task is
            paused.
          </p>
        </div>

        <div className="card flex flex-col items-center justify-center gap-3">
          <div className="flex gap-3">
            <ScoreRing value={snapshot.work_hours.productivity} label="Productivity" />
            <ScoreRing value={snapshot.score.alignment} label="Alignment" />
            <ScoreRing value={snapshot.score.focus_quality} label="Focus" />
          </div>
          <p className="text-center text-2xs leading-relaxed text-ink-500">
            Productivity includes idle time to reveal workday gaps; alignment measures whether
            non-idle activity matched your commitment.
          </p>
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value, cls }: { label: string; value: string; cls: string }) {
  return (
    <div>
      <p className="text-2xs uppercase tracking-wider text-ink-500">{label}</p>
      <p className={`font-mono text-sm ${cls}`}>{value}</p>
    </div>
  );
}

function ActiveCommitmentCard({
  commitment,
  focusElapsed,
  focusedSecs,
  onError,
}: {
  commitment: Commitment;
  focusElapsed: number;
  focusedSecs: number;
  onError: (message: string | null) => void;
}) {
  const { refreshSnapshot, setModal } = useStore();
  const [busyAction, setBusyAction] = useState<"complete" | "pause" | null>(null);
  const est = (commitment.estimated_minutes ?? 0) * 60;
  const perform = async (kind: "complete" | "pause", action: () => Promise<unknown>) => {
    if (busyAction) return;
    setBusyAction(kind);
    onError(null);
    try {
      await action();
      await refreshSnapshot();
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setBusyAction(null);
    }
  };
  return (
    <div className="card border-focus/30">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="section-title text-focus">Active commitment</p>
          <h2 className="mt-1 truncate text-xl font-semibold text-ink-50">{commitment.title}</h2>
          {commitment.done_definition && (
            <p className="mt-1 text-xs text-ink-400">DONE = {commitment.done_definition}</p>
          )}
        </div>
        <div className="text-right">
          <p className="font-mono text-3xl tabular-nums text-ink-50">
            {fmtClockDuration(focusElapsed)}
          </p>
          <p className="text-2xs text-ink-500">this focus session</p>
        </div>
      </div>
      {est > 0 && (
        <div className="mt-3">
          <ProgressBar value={focusedSecs} max={est} />
          <p className="mt-1 text-2xs text-ink-500">
            {fmtDuration(focusedSecs)} focused of ~{fmtDuration(est)} estimated
          </p>
        </div>
      )}
      <CommitmentSteps commitment={commitment} />
      <div className="mt-4 flex gap-2">
        <button
          className="btn btn-primary"
          disabled={busyAction != null}
          onClick={() => void perform("complete", () => api.completeCommitment(commitment.id))}
        >
          {busyAction === "complete" ? "Completing…" : "Complete"}
        </button>
        <button
          className="btn"
          disabled={busyAction != null}
          onClick={() => void perform("pause", () => api.pauseFocus())}
        >
          {busyAction === "pause" ? "Pausing…" : "Pause"}
        </button>
        <button className="btn" onClick={() => setModal({ kind: "blocked", commitmentId: commitment.id })}>
          Blocked
        </button>
        <button
          className="btn"
          onClick={() => setModal({ kind: "switch", fromCommitmentId: commitment.id })}
        >
          Switch priority
        </button>
      </div>
    </div>
  );
}

function CommitmentRow({
  commitment,
  activeId = null,
  onError,
}: {
  commitment: Commitment;
  activeId?: number | null;
  onError: (message: string | null) => void;
}) {
  const { refreshSnapshot, snapshot, setModal } = useStore();
  const [starting, setStarting] = useState(false);
  const [showSteps, setShowSteps] = useState(false);
  const done = commitment.status === "completed";
  const isActive = commitment.id === activeId;
  const pausedContractId =
    activeId == null ? snapshot?.commitments.find((c) => c.status === "active")?.id ?? null : null;
  const contractId = activeId ?? pausedContractId;
  const isPausedContract = !isActive && commitment.id === pausedContractId;
  const isTerminal = ["completed", "deferred", "dropped", "cancelled"].includes(
    commitment.status,
  );
  const requiresSwitch = contractId != null && !isPausedContract;
  const focused =
    snapshot?.commitment_progress.find((p) => p.commitment_id === commitment.id)?.focused_secs ?? 0;
  return (
    <div
      className={`rounded-md border px-3 py-2 ${
        isActive ? "border-focus/40 bg-focus/5" : "border-ink-700 bg-ink-850"
      }`}
    >
      <div className="flex items-center gap-3">
        <span className="w-4 text-center font-mono text-xs text-ink-500">{commitment.rank}</span>
        <div className="min-w-0 flex-1">
          <p className={`truncate text-[13px] ${done ? "text-ink-500 line-through" : "text-ink-100"}`}>
            {commitment.title}
          </p>
          <p className="text-2xs text-ink-500">
            {fmtDuration(focused)} focused
            {commitment.estimated_minutes ? ` · est ${commitment.estimated_minutes}m` : ""}
            {commitment.status === "deferred" && " · deferred"}
            {commitment.status === "cancelled" && " · cancelled"}
            {commitment.steps.length > 0 &&
              ` · ${commitment.steps.filter((step) => step.completed).length}/${commitment.steps.length} steps`}
          </p>
        </div>
        {!isTerminal && (
          <button
            className="btn btn-ghost shrink-0 px-2 py-1 text-accent"
            aria-expanded={showSteps}
            onClick={() => setShowSteps((open) => !open)}
          >
            {showSteps
              ? "Hide steps"
              : commitment.steps.length > 0
                ? "Show steps"
                : "Break into steps"}
          </button>
        )}
        <PriorityTag priority={commitment.priority} />
        {done ? (
          <span className="text-xs text-focus">Done</span>
        ) : isActive ? (
          <span className="text-xs text-focus">Active</span>
        ) : isTerminal ? null : requiresSwitch ? (
          <button
            className="btn py-1"
            onClick={() => setModal({ kind: "switch", fromCommitmentId: contractId })}
          >
            Switch
          </button>
        ) : (
          <button
            className="btn py-1"
            disabled={starting}
            onClick={async () => {
              if (activeId != null && activeId !== commitment.id) {
                // Another commitment is active: switching must be intentional —
                // collect the reason + disposition (spec §7, §15).
                setModal({
                  kind: "switch",
                  fromCommitmentId: activeId,
                  toCommitmentId: commitment.id,
                });
                return;
              }
              if (starting) return;
              setStarting(true);
              onError(null);
              try {
                await api.startCommitment(commitment.id);
                await refreshSnapshot();
              } catch (error) {
                onError(errorMessage(error));
              } finally {
                setStarting(false);
              }
            }}
          >
            {starting ? "Starting…" : isPausedContract ? "Resume" : "Start"}
          </button>
        )}
      </div>
      {showSteps && (
        <CommitmentSteps
          commitment={commitment}
          initiallyBreakingDown={commitment.steps.length === 0}
        />
      )}
    </div>
  );
}
