// Tasks — standard backlog (spec §4 Tasks).

import { useCallback, useEffect, useState } from "react";

import { EmptyState, PriorityTag } from "@/components/shared";
import {
  api,
  errorMessage,
  type Priority,
  type Project,
  type Task,
  type TaskStatus,
} from "@/lib/ipc";

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
  const [tasks, setTasks] = useState<Task[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [filter, setFilter] = useState<TaskStatus | "open">("open");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Task | null>(null);

  const reload = useCallback(async () => {
    try {
      const status = filter === "open" ? null : filter;
      const [t, p] = await Promise.all([api.listTasks(status, search || null), api.listProjects()]);
      setTasks(t);
      setProjects(p);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [filter, search]);

  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-6">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-ink-50">Tasks</h1>
        <input
          className="input w-64"
          placeholder="Search tasks…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      <QuickAdd projects={projects} onAdded={reload} />

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

      {error && <p className="text-xs text-distracted">{error}</p>}

      {tasks.length === 0 ? (
        <EmptyState
          title="No tasks here"
          hint="Capture anything with the field above, or press Ctrl+Shift+Space from anywhere."
        />
      ) : (
        <div className="divide-y divide-ink-800 rounded-lg border border-ink-700 bg-ink-900">
          {tasks.map((t) => (
            <TaskRow
              key={t.id}
              task={t}
              projects={projects}
              onChanged={reload}
              editing={editing?.id === t.id}
              setEditing={(open) => setEditing(open ? t : null)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function QuickAdd({ projects, onAdded }: { projects: Project[]; onAdded: () => Promise<void> }) {
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("should");
  const [projectId, setProjectId] = useState<number | "">("");
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    if (!title.trim() || busy) return;
    setBusy(true);
    try {
      await api.createTask({
        title: title.trim(),
        priority,
        project_id: projectId === "" ? null : projectId,
        status: "inbox",
      });
      setTitle("");
      await onAdded();
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
      <button className="btn btn-primary" onClick={() => void submit()} disabled={!title.trim()}>
        Add
      </button>
    </div>
  );
}

function TaskRow({
  task,
  projects,
  onChanged,
  editing,
  setEditing,
}: {
  task: Task;
  projects: Project[];
  onChanged: () => Promise<void>;
  editing: boolean;
  setEditing: (open: boolean) => void;
}) {
  const project = projects.find((p) => p.id === task.project_id);
  const done = task.status === "completed";
  return (
    <div className="px-3 py-2">
      <div className="flex items-center gap-3">
        <input
          type="checkbox"
          className="h-3.5 w-3.5 accent-[#4ea87c]"
          checked={done}
          onChange={async (e) => {
            await api.setTaskStatus(task.id, e.target.checked ? "completed" : "planned");
            await onChanged();
          }}
        />
        <button className="min-w-0 flex-1 text-left" onClick={() => setEditing(!editing)}>
          <span className={`block truncate text-[13px] ${done ? "text-ink-500 line-through" : "text-ink-100"}`}>
            {task.title}
          </span>
          <span className="text-2xs text-ink-500">
            {task.status}
            {project && ` · ${project.name}`}
            {task.due_date && ` · due ${task.due_date}`}
            {task.estimated_minutes != null && ` · ~${task.estimated_minutes}m`}
            {task.tags.length > 0 && ` · ${task.tags.join(", ")}`}
          </span>
        </button>
        <PriorityTag priority={task.priority} />
        <button
          className="btn-ghost btn px-2 py-1 text-ink-500 hover:text-distracted"
          title="Delete task"
          onClick={async () => {
            await api.deleteTask(task.id);
            await onChanged();
          }}
        >
          ✕
        </button>
      </div>
      {editing && <TaskEditor task={task} projects={projects} onSaved={async () => { setEditing(false); await onChanged(); }} />}
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
