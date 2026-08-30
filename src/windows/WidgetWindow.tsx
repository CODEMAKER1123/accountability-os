// Always-on-top task and productivity widget (spec §25).

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  api,
  errorMessage,
  type Commitment,
  type Project,
  type Task,
  type TodaySnapshot,
} from "@/lib/ipc";
import { flattenTaskHierarchy } from "@/lib/taskBreakdown";
import { fmtClockDuration, fmtDuration, fmtPct } from "@/lib/time";

type TaskView = "today" | "all";

interface SwitchTarget {
  title: string;
  task: Task | null;
  commitment: Commitment | null;
  fromCommitmentId: number;
}

const TERMINAL_COMMITMENT_STATUSES = ["completed", "deferred", "cancelled", "dropped"];
const TERMINAL_TASK_STATUSES = ["completed", "cancelled"];

const SWITCH_DISPOSITIONS: [string, string][] = [
  ["still_today", "Keep current task today"],
  ["later", "Move current task to later"],
  ["defer", "Defer current task"],
  ["cancel", "Cancel current task"],
];

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

function commitmentState(
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

function taskStatusLabel(task: Task): string {
  if (task.status === "inbox") return "Inbox";
  return task.status.charAt(0).toUpperCase() + task.status.slice(1);
}

export default function WidgetWindow() {
  const [snap, setSnap] = useState<TodaySnapshot | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [view, setView] = useState<TaskView>("today");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [switchTarget, setSwitchTarget] = useState<SwitchTarget | null>(null);
  const [switchReason, setSwitchReason] = useState("");
  const [switchDisposition, setSwitchDisposition] = useState("still_today");
  const busy = useRef(false);
  const [, force] = useState(0);

  const load = useCallback(async () => {
    try {
      const [nextSnapshot, nextTasks, nextProjects] = await Promise.all([
        api.getTodaySnapshot(),
        api.listTasks(null, null),
        api.listProjects(),
      ]);
      setSnap(nextSnapshot);
      setTasks(nextTasks);
      setProjects(nextProjects);
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, []);

  const run = async (key: string, action: () => Promise<unknown>): Promise<boolean> => {
    if (busy.current) return false;
    busy.current = true;
    setBusyKey(key);
    setError(null);
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      busy.current = false;
      setBusyKey(null);
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
  const currentContractId = activeId ?? pausedId;
  const elapsed = snap?.focus_session ? now - snap.focus_session.started_at : 0;
  const onBreak = Boolean(snap?.current_break && snap.current_break.ends_at > now);

  const todayCommitmentByTask = useMemo(() => {
    const result = new Map<number, Commitment>();
    for (const commitment of snap?.commitments ?? []) {
      if (commitment.task_id == null) continue;
      const current = result.get(commitment.task_id);
      const currentIsClosed =
        current != null && TERMINAL_COMMITMENT_STATUSES.includes(current.status);
      const nextIsOpen = !TERMINAL_COMMITMENT_STATUSES.includes(commitment.status);
      if (current == null || (currentIsClosed && nextIsOpen)) {
        result.set(commitment.task_id, commitment);
      }
    }
    return result;
  }, [snap?.commitments]);

  const projectById = useMemo(
    () => new Map(projects.map((project) => [project.id, project])),
    [projects],
  );
  const taskRows = useMemo(() => flattenTaskHierarchy(tasks), [tasks]);
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const visibleTaskRows = taskRows.filter(({ task }) => {
    if (!normalizedSearch) return true;
    const project = task.project_id == null ? null : projectById.get(task.project_id);
    return [task.title, task.description, project?.name ?? "", ...task.tags]
      .join(" ")
      .toLocaleLowerCase()
      .includes(normalizedSearch);
  });

  const requestSwitch = (
    title: string,
    task: Task | null,
    commitment: Commitment | null,
  ) => {
    if (currentContractId == null) return;
    setSwitchReason("");
    setSwitchDisposition("still_today");
    setSwitchTarget({ title, task, commitment, fromCommitmentId: currentContractId });
  };

  const startTask = async (task: Task, commitment: Commitment | null) => {
    if (onBreak) return;
    if (commitment && TERMINAL_COMMITMENT_STATUSES.includes(commitment.status)) return;
    if (currentContractId != null && commitment?.id !== currentContractId) {
      requestSwitch(task.title, task, commitment);
      return;
    }
    await run(`task-${task.id}`, async () => {
      const prepared = commitment ?? (await api.prepareTaskForToday(task.id));
      await api.startCommitment(prepared.id);
    });
  };

  const startCommitment = async (commitment: Commitment) => {
    if (onBreak) return;
    if (currentContractId != null && commitment.id !== currentContractId) {
      requestSwitch(commitment.title, null, commitment);
      return;
    }
    await run(`commitment-${commitment.id}`, () => api.startCommitment(commitment.id));
  };

  const submitSwitch = async () => {
    if (!switchTarget || switchReason.trim().length < 3) return;
    const target = switchTarget;
    const switched = await run(
      `switch-${target.task?.id ?? target.commitment?.id ?? "target"}`,
      async () => {
        await api.switchCommitment({
          from_commitment_id: target.fromCommitmentId,
          to_commitment_id: target.commitment?.id ?? null,
          to_task_id: target.commitment == null ? target.task!.id : null,
          reason: switchReason.trim(),
          original_disposition: switchDisposition,
        });
      },
    );
    if (switched) setSwitchTarget(null);
  };

  const beginResize = (event: React.PointerEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    void getCurrentWindow()
      .startResizeDragging("SouthEast")
      .catch((caught) => setError(`Could not resize the widget: ${errorMessage(caught)}`));
  };

  return (
    <div
      className="relative flex h-screen min-h-0 select-none flex-col overflow-hidden border border-ink-600 bg-ink-950 p-3 pb-4"
      data-tauri-drag-region
    >
      <div className="flex items-center justify-between" data-tauri-drag-region>
        <span className="text-2xs font-bold uppercase tracking-widest text-ink-400">
          Accountability OS
        </span>
        <div className="flex items-center gap-2">
          <span className="text-[9px] text-ink-600" title="Drag the bottom-right corner to resize">
            Drag corner ↘
          </span>
          <button
            className="text-ink-600 hover:text-ink-300"
            title="Close widget"
            onClick={() => void run("close", () => api.setWidgetVisible(false))}
          >
            ✕
          </button>
        </div>
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
                disabled={busyKey != null}
                onClick={() => void run("break", () => api.endBreakNow())}
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
                disabled={busyKey != null}
                onClick={() =>
                  void run(`complete-${active.id}`, () => api.completeCommitment(active.id))
                }
              >
                Done
              </button>
              <button
                className="btn py-1 text-2xs"
                disabled={busyKey != null}
                onClick={() => void run(`pause-${active.id}`, () => api.pauseFocus())}
              >
                Pause
              </button>
            </div>
          ) : (
            <p className="mt-2 rounded-md border border-ink-700 bg-ink-900 px-3 py-2 text-2xs text-ink-400">
              Focus is paused. Desktop activity monitoring continues independently.
            </p>
          )}

          <div className="mt-2 flex min-h-0 flex-1 flex-col">
            <div className="mb-1 flex items-center justify-between gap-2">
              <div className="flex rounded-md border border-ink-700 bg-ink-900 p-0.5" role="tablist">
                <button
                  className={`rounded px-2 py-1 text-2xs ${
                    view === "today" ? "bg-ink-700 text-ink-50" : "text-ink-400"
                  }`}
                  role="tab"
                  aria-selected={view === "today"}
                  onClick={() => setView("today")}
                >
                  Today ({snap.commitments.length})
                </button>
                <button
                  className={`rounded px-2 py-1 text-2xs ${
                    view === "all" ? "bg-ink-700 text-ink-50" : "text-ink-400"
                  }`}
                  role="tab"
                  aria-selected={view === "all"}
                  onClick={() => setView("all")}
                >
                  All tasks ({taskRows.length})
                </button>
              </div>
              <span className="text-2xs text-ink-600">
                {view === "today" ? `${snap.commitments.length}/3 outcomes` : "Open backlog"}
              </span>
            </div>

            {view === "all" && (
              <input
                className="input mb-1.5 h-8 select-text py-1 text-xs"
                placeholder="Search every task…"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            )}

            <div className="min-h-0 flex-1 overflow-y-auto pr-1" role="tabpanel">
              {view === "today" ? (
                snap.commitments.length === 0 ? (
                  <p className="rounded-md border border-ink-700 px-3 py-2 text-xs text-ink-400">
                    No plan yet. Choose All tasks to start from your backlog.
                  </p>
                ) : (
                  <ol className="space-y-1">
                    {snap.commitments.map((commitment) => (
                      <CommitmentRow
                        key={commitment.id}
                        commitment={commitment}
                        activeId={activeId}
                        pausedId={pausedId}
                        currentContractId={currentContractId}
                        onBreak={onBreak}
                        busy={busyKey != null}
                        busyLabel={busyKey === `commitment-${commitment.id}`}
                        onStart={() => void startCommitment(commitment)}
                        onSwitch={() => requestSwitch(commitment.title, null, commitment)}
                      />
                    ))}
                  </ol>
                )
              ) : visibleTaskRows.length === 0 ? (
                <p className="rounded-md border border-ink-700 px-3 py-2 text-xs text-ink-400">
                  {search ? "No tasks match this search." : "No open tasks yet."}
                </p>
              ) : (
                <ol className="space-y-1">
                  {visibleTaskRows.map(({ task, depth, directChildren }) => (
                    <TaskRow
                      key={task.id}
                      task={task}
                      depth={depth}
                      childCount={directChildren.length}
                      project={
                        task.project_id == null ? null : projectById.get(task.project_id) ?? null
                      }
                      commitment={todayCommitmentByTask.get(task.id) ?? null}
                      activeId={activeId}
                      pausedId={pausedId}
                      currentContractId={currentContractId}
                      onBreak={onBreak}
                      busy={busyKey != null}
                      busyLabel={busyKey === `task-${task.id}`}
                      onStart={() =>
                        void startTask(task, todayCommitmentByTask.get(task.id) ?? null)
                      }
                    />
                  ))}
                </ol>
              )}
            </div>
          </div>

          <button
            className="btn mt-2 py-1 text-2xs"
            disabled={busyKey != null}
            onClick={() => void run("open", () => api.showMainWindow())}
          >
            Open app
          </button>
        </>
      )}

      <button
        type="button"
        className="absolute bottom-0 right-0 z-20 h-5 w-5 cursor-se-resize text-ink-500 hover:text-accent"
        aria-label="Resize widget"
        title="Drag to resize"
        onPointerDown={beginResize}
      >
        <svg className="h-full w-full" viewBox="0 0 20 20" fill="none" aria-hidden="true">
          <path d="M9 17h8V9M13 17h4v-4" stroke="currentColor" strokeWidth="1.5" />
        </svg>
      </button>

      {switchTarget && (
        <form
          className="absolute inset-2 z-30 flex flex-col rounded-lg border border-ink-600 bg-ink-900 p-3 shadow-2xl"
          onSubmit={(event) => {
            event.preventDefault();
            void submitSwitch();
          }}
        >
          <p className="text-sm font-semibold text-ink-50">Switch priority</p>
          <p className="mt-1 line-clamp-2 text-xs text-ink-400">Start: {switchTarget.title}</p>
          <label className="label mt-3" htmlFor="widget-switch-reason">
            What changed?
          </label>
          <input
            id="widget-switch-reason"
            autoFocus
            className="input select-text"
            placeholder="Required reason"
            value={switchReason}
            onChange={(event) => setSwitchReason(event.target.value)}
          />
          <label className="label mt-3" htmlFor="widget-switch-disposition">
            Current task
          </label>
          <select
            id="widget-switch-disposition"
            className="input select-text"
            value={switchDisposition}
            onChange={(event) => setSwitchDisposition(event.target.value)}
          >
            {SWITCH_DISPOSITIONS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
          {error && <p className="mt-2 line-clamp-3 text-2xs text-distracted">{error}</p>}
          <div className="mt-auto flex justify-end gap-2 pt-3">
            <button
              type="button"
              className="btn"
              disabled={busyKey != null}
              onClick={() => setSwitchTarget(null)}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="btn btn-primary"
              disabled={busyKey != null || switchReason.trim().length < 3}
            >
              {busyKey?.startsWith("switch-") ? "Switching…" : "Switch"}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}

function CommitmentRow({
  commitment,
  activeId,
  pausedId,
  currentContractId,
  onBreak,
  busy,
  busyLabel,
  onStart,
  onSwitch,
}: {
  commitment: Commitment;
  activeId: number | null;
  pausedId: number | null;
  currentContractId: number | null;
  onBreak: boolean;
  busy: boolean;
  busyLabel: boolean;
  onStart: () => void;
  onSwitch: () => void;
}) {
  const state = commitmentState(commitment, activeId, pausedId);
  const terminal = TERMINAL_COMMITMENT_STATUSES.includes(commitment.status);
  const active = commitment.id === activeId;
  const requiresSwitch = currentContractId != null && currentContractId !== commitment.id;
  return (
    <li className="flex items-center gap-2 rounded-md border border-ink-700 bg-ink-900 px-2.5 py-1.5">
      <span className="w-3 font-mono text-2xs text-ink-600">{commitment.rank}.</span>
      <div className="min-w-0 flex-1">
        <p
          className={`line-clamp-2 text-xs ${
            commitment.status === "completed" ? "text-ink-500 line-through" : "text-ink-100"
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
      {!terminal && !active && (
        <button
          className="btn shrink-0 py-1 text-2xs"
          disabled={busy || onBreak}
          title={onBreak ? "End the current break before starting work." : undefined}
          onClick={requiresSwitch ? onSwitch : onStart}
        >
          {onBreak
            ? "On break"
            : busyLabel
              ? "Starting…"
              : requiresSwitch
                ? "Switch"
                : commitment.id === pausedId
                  ? "Resume"
                  : "Start"}
        </button>
      )}
    </li>
  );
}

function TaskRow({
  task,
  depth,
  childCount,
  project,
  commitment,
  activeId,
  pausedId,
  currentContractId,
  onBreak,
  busy,
  busyLabel,
  onStart,
}: {
  task: Task;
  depth: number;
  childCount: number;
  project: Project | null;
  commitment: Commitment | null;
  activeId: number | null;
  pausedId: number | null;
  currentContractId: number | null;
  onBreak: boolean;
  busy: boolean;
  busyLabel: boolean;
  onStart: () => void;
}) {
  const taskTerminal = TERMINAL_TASK_STATUSES.includes(task.status);
  const closedToday =
    commitment != null && TERMINAL_COMMITMENT_STATUSES.includes(commitment.status);
  const active = commitment?.id === activeId;
  const paused = commitment?.id === pausedId;
  const requiresSwitch = currentContractId != null && commitment?.id !== currentContractId;
  const state = active
    ? "Active"
    : paused
      ? "Paused"
      : closedToday
        ? commitment.status === "deferred"
          ? "Deferred today"
          : "Closed today"
        : taskStatusLabel(task);
  return (
    <li
      className="flex items-center gap-2 rounded-md border border-ink-700 bg-ink-900 py-1.5 pr-2.5"
      style={{ paddingLeft: `${10 + Math.min(depth, 5) * 16}px` }}
    >
      <div className="min-w-0 flex-1">
        <p
          className={`line-clamp-2 text-xs ${taskTerminal ? "text-ink-500 line-through" : "text-ink-100"}`}
          title={task.title}
        >
          {task.title}
        </p>
        <p className="truncate text-2xs text-ink-500">
          {state}
          {project ? ` · ${project.name}` : ""}
          {childCount > 0 ? ` · ${childCount} steps` : ""}
        </p>
      </div>
      {!taskTerminal && !closedToday && !active && (
        <button
          className="btn shrink-0 py-1 text-2xs"
          disabled={busy || onBreak}
          title={onBreak ? "End the current break before starting work." : undefined}
          onClick={onStart}
        >
          {onBreak
            ? "On break"
            : busyLabel
              ? "Starting…"
              : requiresSwitch
                ? "Switch"
                : paused
                  ? "Resume"
                  : "Start"}
        </button>
      )}
    </li>
  );
}
