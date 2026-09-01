// Tasks — standard backlog (spec §4 Tasks).

import { useCallback, useEffect, useRef, useState } from "react";

import { BreakdownEditor } from "@/components/BreakdownEditor";
import { EmptyState, ErrorBanner, PriorityTag } from "@/components/shared";
import {
  api,
  errorMessage,
  type Commitment,
  type Priority,
  type Project,
  type Task,
  type TaskStatus,
} from "@/lib/ipc";
import { useStore } from "@/lib/store";
import { flattenTaskHierarchy } from "@/lib/taskBreakdown";

const STATUS_FILTERS: { id: TaskStatus | "open"; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "inbox", label: "Inbox" },
  { id: "planned", label: "Planned" },
  { id: "committed", label: "Committed" },
  { id: "active", label: "Active" },
  { id: "completed", label: "Completed" },
  { id: "deferred", label: "Deferred" },
  { id: "cancelled", label: "Cancelled" },
];

export default function Tasks() {
  const { snapshot, refreshSnapshot, setModal } = useStore();
  const [tasks, setTasks] = useState<Task[]>([]);
  const [allOpenTasks, setAllOpenTasks] = useState<Task[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [filter, setFilter] = useState<TaskStatus | "open">("open");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Task | null>(null);
  const [showProjects, setShowProjects] = useState(false);
  const [startingTaskId, setStartingTaskId] = useState<number | null>(null);
  const startInProgress = useRef(false);

  const reload = useCallback(async () => {
    try {
      const status = filter === "open" ? null : filter;
      const visibleTasks = api.listTasks(status, search || null);
      const taskContext = status == null && !search ? visibleTasks : api.listTasks(null, null);
      const [t, p, all] = await Promise.all([visibleTasks, api.listProjects(), taskContext]);
      setTasks(t);
      setAllOpenTasks(all);
      setProjects(p);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [filter, search]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const taskRows = flattenTaskHierarchy(tasks);
  const allChildrenByParent = new Map(
    flattenTaskHierarchy(allOpenTasks).map(({ task, directChildren }) => [
      task.id,
      directChildren,
    ]),
  );
  const todayCommitmentByTask = new Map<number, Commitment>();
  for (const commitment of snapshot?.commitments ?? []) {
    if (commitment.task_id == null) continue;
    const current = todayCommitmentByTask.get(commitment.task_id);
    const currentIsClosed =
      current != null && ["completed", "deferred", "dropped", "cancelled"].includes(current.status);
    const nextIsOpen = !["completed", "deferred", "dropped", "cancelled"].includes(
      commitment.status,
    );
    if (current == null || (currentIsClosed && nextIsOpen)) {
      todayCommitmentByTask.set(commitment.task_id, commitment);
    }
  }
  const searchTerm = search.trim().toLowerCase();
  const visibleUnlinkedToday =
    filter === "open"
      ? (snapshot?.commitments ?? []).filter(
          (commitment) =>
            commitment.task_id == null &&
            !["completed", "deferred", "dropped", "cancelled"].includes(commitment.status) &&
            (!searchTerm || commitment.title.toLowerCase().includes(searchTerm)),
        )
      : [];
  const activeCommitmentId = snapshot?.active_commitment?.id ?? null;
  const pausedCommitmentId =
    activeCommitmentId == null
      ? snapshot?.commitments.find((commitment) => commitment.status === "active")?.id ?? null
      : null;
  const currentContractId = activeCommitmentId ?? pausedCommitmentId;
  const now = Math.floor(Date.now() / 1000);
  const onBreak = Boolean(snapshot?.current_break && snapshot.current_break.ends_at > now);
  const hasVisibleRows = tasks.length > 0 || visibleUnlinkedToday.length > 0;

  const startTask = async (task: Task, existingCommitment: Commitment | null) => {
    if (startInProgress.current || onBreak) return;
    startInProgress.current = true;
    setStartingTaskId(task.id);
    setError(null);
    try {
      if (
        existingCommitment &&
        ["completed", "deferred", "dropped", "cancelled"].includes(existingCommitment.status)
      ) {
        throw new Error("That task is already closed in today's accountability record.");
      }

      const beforePrepare = useStore.getState().snapshot;
      const beforeActiveId = beforePrepare?.active_commitment?.id ?? null;
      const beforePausedId =
        beforeActiveId == null
          ? beforePrepare?.commitments.find((item) => item.status === "active")?.id ?? null
          : null;
      const beforeContractId = beforeActiveId ?? beforePausedId;
      if (beforeContractId != null && existingCommitment?.id !== beforeContractId) {
        setModal({
          kind: "switch",
          fromCommitmentId: beforeContractId,
          toCommitmentId: existingCommitment?.id ?? null,
          toTaskId: existingCommitment == null ? task.id : null,
          toTaskTitle: existingCommitment == null ? task.title : undefined,
        });
        return;
      }

      if (existingCommitment) {
        await api.startCommitment(existingCommitment.id);
      } else {
        await api.startTask(task.id);
      }
      await Promise.all([reload(), refreshSnapshot()]);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      startInProgress.current = false;
      setStartingTaskId(null);
    }
  };

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-lg font-semibold text-ink-50">Tasks</h1>
        <div className="flex items-center gap-2">
          <button className="btn" onClick={() => setShowProjects((visible) => !visible)}>
            {showProjects ? "Hide projects" : `Projects${projects.length ? ` (${projects.length})` : ""}`}
          </button>
          <input
            className="input w-64"
            placeholder="Search tasks…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
      </div>

      {showProjects && <ProjectPanel projects={projects} onChanged={reload} />}

      <QuickAdd projects={projects} onAdded={reload} onError={setError} />

      <div className="flex gap-1">
        {STATUS_FILTERS.map((f) => (
          <button
            key={f.id}
            onClick={() => setFilter(f.id)}
            className={`rounded px-2 py-1 text-xs ${
              filter === f.id ? "bg-ink-700 text-ink-50" : "text-ink-400 hover:text-ink-200"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>

      {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

      {!hasVisibleRows ? (
        <EmptyState
          title="No tasks here"
          hint={
            search
              ? "No tasks match this search."
              : filter === "open"
                ? "Add your first task above, or create a project to organize related work."
                : `There are no ${filter} tasks. Choose Open to see everything still actionable.`
          }
        />
      ) : (
        <>
          {visibleUnlinkedToday.length > 0 && (
            <section className="rounded-lg border border-focus/30 bg-ink-900 p-3">
              <div className="mb-2">
                <p className="section-title text-focus">Today&apos;s commitments</p>
                <p className="mt-1 text-xs text-ink-400">
                  Outcomes from today&apos;s plan that are not linked to a backlog task yet.
                </p>
              </div>
              <div className="space-y-2">
                {visibleUnlinkedToday.map((commitment) => (
                  <TodayCommitmentRow
                    key={commitment.id}
                    commitment={commitment}
                    activeCommitmentId={activeCommitmentId}
                    pausedCommitmentId={pausedCommitmentId}
                    currentContractId={currentContractId}
                    onBreak={onBreak}
                    onChanged={async () => {
                      await Promise.all([reload(), refreshSnapshot()]);
                    }}
                    onError={setError}
                  />
                ))}
              </div>
            </section>
          )}
          {tasks.length > 0 && (
            <div className="divide-y divide-ink-800 rounded-lg border border-ink-700 bg-ink-900">
              {taskRows.map(({ task, depth, directChildren }) => (
                <TaskRow
                  key={task.id}
                  task={task}
                  depth={depth}
                  childTasks={allChildrenByParent.get(task.id) ?? directChildren}
                  projects={projects}
                  todayCommitment={todayCommitmentByTask.get(task.id) ?? null}
                  activeCommitmentId={activeCommitmentId}
                  pausedCommitmentId={pausedCommitmentId}
                  currentContractId={currentContractId}
                  onBreak={onBreak}
                  starting={startingTaskId === task.id}
                  startBusy={startingTaskId != null}
                  onStart={startTask}
                  onChanged={reload}
                  editing={editing?.id === task.id}
                  setEditing={(open) => setEditing(open ? task : null)}
                  onError={setError}
                />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function ProjectPanel({
  projects,
  onChanged,
}: {
  projects: Project[];
  onChanged: () => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [color, setColor] = useState("#4ea87c");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const create = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.createProject(name.trim(), color);
      setName("");
      await onChanged();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card space-y-3" aria-labelledby="projects-heading">
      <div>
        <p id="projects-heading" className="section-title">
          Projects
        </p>
        <p className="mt-1 text-xs text-ink-400">
          Group related backlog tasks, then choose the project while adding or editing a task.
        </p>
      </div>
      <div className="flex gap-2">
        <input
          className="input flex-1"
          placeholder="New project name…"
          value={name}
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => event.key === "Enter" && void create()}
        />
        <label className="flex items-center gap-2 rounded-md border border-ink-700 px-2 text-xs text-ink-400">
          Color
          <input
            className="h-6 w-8 cursor-pointer border-0 bg-transparent p-0"
            type="color"
            value={color}
            onChange={(event) => setColor(event.target.value)}
          />
        </label>
        <button
          className="btn btn-primary"
          disabled={!name.trim() || busy}
          onClick={() => void create()}
        >
          {busy ? "Creating…" : "Create project"}
        </button>
      </div>
      {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}
      {projects.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {projects.map((project) => (
            <span
              key={project.id}
              className="inline-flex items-center gap-2 rounded-full border border-ink-700 bg-ink-850 px-2.5 py-1 text-xs text-ink-200"
            >
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: project.color ?? "#697684" }}
              />
              {project.name}
            </span>
          ))}
        </div>
      ) : (
        <p className="text-xs text-ink-500">No projects yet.</p>
      )}
    </section>
  );
}

function QuickAdd({
  projects,
  onAdded,
  onError,
}: {
  projects: Project[];
  onAdded: () => Promise<void>;
  onError: (message: string | null) => void;
}) {
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("should");
  const [projectId, setProjectId] = useState<number | "">("");
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    if (!title.trim() || busy) return;
    setBusy(true);
    onError(null);
    try {
      await api.createTask({
        title: title.trim(),
        priority,
        project_id: projectId === "" ? null : projectId,
        status: "inbox",
      });
      setTitle("");
      await onAdded();
    } catch (caught) {
      onError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex gap-2">
      <input
        className="input flex-1"
        placeholder="Add a task… (Enter to save)"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && void submit()}
      />
      <select
        className="input w-28"
        value={priority}
        onChange={(e) => setPriority(e.target.value as Priority)}
      >
        <option value="must">Must</option>
        <option value="should">Should</option>
        <option value="could">Could</option>
      </select>
      <select
        className="input w-40"
        value={projectId}
        onChange={(e) => setProjectId(e.target.value === "" ? "" : Number(e.target.value))}
      >
        <option value="">No project</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
      <button
        className="btn btn-primary"
        onClick={() => void submit()}
        disabled={!title.trim() || busy}
      >
        {busy ? "Adding…" : "Add"}
      </button>
    </div>
  );
}

function TodayCommitmentRow({
  commitment,
  activeCommitmentId,
  pausedCommitmentId,
  currentContractId,
  onBreak,
  onChanged,
  onError,
}: {
  commitment: Commitment;
  activeCommitmentId: number | null;
  pausedCommitmentId: number | null;
  currentContractId: number | null;
  onBreak: boolean;
  onChanged: () => Promise<void>;
  onError: (message: string | null) => void;
}) {
  const { setModal } = useStore();
  const [busy, setBusy] = useState<"start" | "complete" | null>(null);
  const terminal = ["completed", "deferred", "dropped", "cancelled"].includes(
    commitment.status,
  );
  const isActive = commitment.id === activeCommitmentId;
  const isPaused = commitment.id === pausedCommitmentId;
  const requiresSwitch = currentContractId != null && currentContractId !== commitment.id;

  const complete = async () => {
    if (busy != null || terminal) return;
    setBusy("complete");
    onError(null);
    try {
      await api.completeCommitment(commitment.id);
      await onChanged();
    } catch (caught) {
      onError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  const start = async () => {
    if (busy != null || terminal || onBreak) return;
    if (requiresSwitch) {
      setModal({
        kind: "switch",
        fromCommitmentId: currentContractId,
        toCommitmentId: commitment.id,
      });
      return;
    }
    setBusy("start");
    onError(null);
    try {
      await api.startCommitment(commitment.id);
      await onChanged();
    } catch (caught) {
      onError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div
      className={`flex items-center gap-3 rounded-md border px-3 py-2 ${
        isActive ? "border-focus/40 bg-focus/5" : "border-ink-700 bg-ink-850"
      }`}
    >
      <div className="min-w-0 flex-1">
        <p className={`truncate text-[13px] ${terminal ? "text-ink-500 line-through" : "text-ink-100"}`}>
          {commitment.title}
        </p>
        <p className="text-2xs text-ink-500">
          {commitment.status} today
          {commitment.steps.length > 0 &&
            ` · ${commitment.steps.filter((step) => step.completed).length}/${commitment.steps.length} steps`}
        </p>
      </div>
      {!terminal && (
        <button
          className="btn btn-primary shrink-0 py-1"
          disabled={busy != null}
          onClick={() => void complete()}
        >
          {busy === "complete" ? "Completing…" : "Done"}
        </button>
      )}
      {!terminal &&
        (isActive ? (
          <span className="shrink-0 text-xs font-medium text-focus">Active</span>
        ) : (
          <button
            className="btn shrink-0 py-1"
            disabled={busy != null || onBreak}
            title={onBreak ? "End the current break before starting work." : undefined}
            onClick={() => void start()}
          >
            {onBreak ? "On break" : busy === "start" ? "Starting…" : isPaused ? "Resume" : requiresSwitch ? "Switch" : "Start"}
          </button>
        ))}
    </div>
  );
}

function TaskRow({
  task,
  depth,
  childTasks,
  projects,
  todayCommitment,
  activeCommitmentId,
  pausedCommitmentId,
  currentContractId,
  onBreak,
  starting,
  startBusy,
  onStart,
  onChanged,
  editing,
  setEditing,
  onError,
}: {
  task: Task;
  depth: number;
  childTasks: Task[];
  projects: Project[];
  todayCommitment: Commitment | null;
  activeCommitmentId: number | null;
  pausedCommitmentId: number | null;
  currentContractId: number | null;
  onBreak: boolean;
  starting: boolean;
  startBusy: boolean;
  onStart: (task: Task, commitment: Commitment | null) => Promise<void>;
  onChanged: () => Promise<void>;
  editing: boolean;
  setEditing: (open: boolean) => void;
  onError: (message: string | null) => void;
}) {
  const { refreshSnapshot } = useStore();
  const [breakingDown, setBreakingDown] = useState(false);
  const project = projects.find((p) => p.id === task.project_id);
  const done = task.status === "completed";
  const terminal = done || task.status === "cancelled";
  const closedToday =
    todayCommitment != null &&
    ["completed", "deferred", "dropped", "cancelled"].includes(todayCommitment.status);
  const focusActive = todayCommitment?.id === activeCommitmentId;
  const focusPaused = todayCommitment?.id === pausedCommitmentId;
  const requiresSwitch = currentContractId != null && todayCommitment?.id !== currentContractId;
  const completedSteps = childTasks.filter((child) => child.status === "completed").length;
  return (
    <div className="px-3 py-2" style={{ paddingLeft: `${12 + Math.min(depth, 5) * 24}px` }}>
      <div className="flex items-center gap-3">
        {depth > 0 && (
          <span className="-ml-1 text-xs text-ink-600" aria-hidden="true">
            ↳
          </span>
        )}
        <input
          type="checkbox"
          className="h-3.5 w-3.5 accent-[#4ea87c]"
          checked={done}
          onChange={async (e) => {
            onError(null);
            try {
              if (e.target.checked && todayCommitment && !closedToday) {
                await api.completeCommitment(todayCommitment.id);
                await Promise.all([onChanged(), refreshSnapshot()]);
                return;
              }
              if (!e.target.checked && todayCommitment && closedToday) {
                onError("Today's completed commitment cannot be reopened from the task list.");
                return;
              }
              await api.setTaskStatus(task.id, e.target.checked ? "completed" : "planned");
              await onChanged();
            } catch (caught) {
              onError(errorMessage(caught));
            }
          }}
        />
        <button className="min-w-0 flex-1 text-left" onClick={() => setEditing(!editing)}>
          <span
            className={`block truncate text-[13px] ${done ? "text-ink-500 line-through" : "text-ink-100"}`}
          >
            {task.title}
          </span>
          <span className="text-2xs text-ink-500">
            {task.status}
            {closedToday && ` · ${todayCommitment.status} today`}
            {project && ` · ${project.name}`}
            {task.due_date && ` · due ${task.due_date}`}
            {task.estimated_minutes != null && ` · ~${task.estimated_minutes}m`}
            {task.tags.length > 0 && ` · ${task.tags.join(", ")}`}
            {childTasks.length > 0 && ` · ${completedSteps}/${childTasks.length} steps`}
          </span>
        </button>
        {!terminal && !closedToday &&
          (focusActive ? (
            <span className="shrink-0 text-xs font-medium text-focus">Active</span>
          ) : (
            <button
              className={`btn shrink-0 py-1 ${requiresSwitch ? "" : "btn-primary"}`}
              disabled={startBusy || onBreak}
              title={onBreak ? "End the current break before starting work." : undefined}
              onClick={() => void onStart(task, todayCommitment)}
            >
              {onBreak
                ? "On break"
                : starting
                  ? "Starting…"
                  : requiresSwitch
                    ? "Switch"
                    : focusPaused
                      ? "Resume"
                      : "Start"}
            </button>
          ))}
        {!terminal && (
          <button
            className="btn btn-ghost shrink-0 px-2 py-1 text-accent"
            aria-expanded={breakingDown}
            onClick={() => {
              setEditing(false);
              setBreakingDown((open) => !open);
            }}
          >
            {breakingDown ? "Close steps" : "Break into steps"}
          </button>
        )}
        <PriorityTag priority={task.priority} />
        <button
          className="btn-ghost btn px-2 py-1 text-ink-500 hover:text-distracted"
          title="Delete task"
          onClick={async () => {
            onError(null);
            try {
              await api.deleteTask(task.id);
              await onChanged();
            } catch (caught) {
              onError(errorMessage(caught));
            }
          }}
        >
          ✕
        </button>
      </div>
      {editing && (
        <TaskEditor
          task={task}
          projects={projects}
          onSaved={async () => {
            setEditing(false);
            await onChanged();
          }}
        />
      )}
      {breakingDown && (
        <BreakdownEditor
          goal={task.title}
          existingSteps={childTasks.map((child) => child.title)}
          onClose={() => setBreakingDown(false)}
          onSave={async (steps) => {
            await api.createTaskSteps(task.id, steps);
            await onChanged();
            setBreakingDown(false);
          }}
        />
      )}
    </div>
  );
}

function TaskEditor({
  task,
  projects,
  onSaved,
}: {
  task: Task;
  projects: Project[];
  onSaved: () => Promise<void>;
}) {
  const [form, setForm] = useState({
    title: task.title,
    description: task.description,
    priority: task.priority as Priority,
    status: task.status as TaskStatus,
    project_id: task.project_id,
    estimated_minutes: task.estimated_minutes,
    due_date: task.due_date ?? "",
    tags: task.tags.join(", "),
  });
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="mt-2 space-y-2 rounded-md border border-ink-700 bg-ink-850 p-3">
      <input
        className="input"
        value={form.title}
        onChange={(e) => setForm({ ...form, title: e.target.value })}
      />
      <textarea
        className="input h-16 resize-none"
        placeholder="Description"
        value={form.description}
        onChange={(e) => setForm({ ...form, description: e.target.value })}
      />
      <div className="grid grid-cols-5 gap-2">
        <select
          className="input"
          value={form.status}
          onChange={(e) => setForm({ ...form, status: e.target.value as TaskStatus })}
        >
          {["inbox", "planned", "committed", "active", "completed", "deferred", "cancelled"].map(
            (s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ),
          )}
        </select>
        <select
          className="input"
          value={form.priority}
          onChange={(e) => setForm({ ...form, priority: e.target.value as Priority })}
        >
          <option value="must">must</option>
          <option value="should">should</option>
          <option value="could">could</option>
        </select>
        <select
          className="input"
          value={form.project_id ?? ""}
          onChange={(e) =>
            setForm({ ...form, project_id: e.target.value === "" ? null : Number(e.target.value) })
          }
        >
          <option value="">No project</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <input
          className="input"
          type="number"
          min={0}
          placeholder="Est. min"
          value={form.estimated_minutes ?? ""}
          onChange={(e) =>
            setForm({
              ...form,
              estimated_minutes: e.target.value === "" ? null : Number(e.target.value),
            })
          }
        />
        <input
          className="input"
          type="date"
          value={form.due_date}
          onChange={(e) => setForm({ ...form, due_date: e.target.value })}
        />
      </div>
      <input
        className="input"
        placeholder="Tags, comma-separated"
        value={form.tags}
        onChange={(e) => setForm({ ...form, tags: e.target.value })}
      />
      {error && <p className="text-xs text-distracted">{error}</p>}
      <div className="flex justify-end gap-2">
        <button
          className="btn btn-primary"
          onClick={async () => {
            try {
              await api.updateTask(task.id, {
                title: form.title,
                description: form.description,
                parent_task_id: task.parent_task_id,
                priority: form.priority,
                status: form.status,
                project_id: form.project_id,
                estimated_minutes: form.estimated_minutes,
                due_date: form.due_date || null,
                tags: form.tags
                  .split(",")
                  .map((t) => t.trim())
                  .filter(Boolean),
              });
              await onSaved();
            } catch (e) {
              setError(errorMessage(e));
            }
          }}
        >
          Save
        </button>
      </div>
    </div>
  );
}
