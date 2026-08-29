import type { Task } from "./ipc";

export const MAX_ACTION_STEPS = 12;

export interface TaskHierarchyRow {
  task: Task;
  depth: number;
  directChildren: Task[];
}

/**
 * Keep the backend's priority ordering while moving child tasks directly
 * beneath their parent. Orphaned rows (for example under a status filter)
 * remain visible as roots instead of disappearing.
 */
export function flattenTaskHierarchy(tasks: Task[]): TaskHierarchyRow[] {
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const children = new Map<number, Task[]>();
  for (const task of tasks) {
    if (task.parent_task_id == null || !byId.has(task.parent_task_id)) continue;
    const siblings = children.get(task.parent_task_id) ?? [];
    siblings.push(task);
    children.set(task.parent_task_id, siblings);
  }

  const rows: TaskHierarchyRow[] = [];
  const visited = new Set<number>();
  const visit = (task: Task, depth: number) => {
    if (visited.has(task.id)) return;
    visited.add(task.id);
    const directChildren = children.get(task.id) ?? [];
    rows.push({ task, depth, directChildren });
    for (const child of directChildren) visit(child, depth + 1);
  };

  for (const task of tasks) {
    if (task.parent_task_id == null || !byId.has(task.parent_task_id)) visit(task, 0);
  }
  // Defensive fallback for corrupt/cyclic imported data.
  for (const task of tasks) visit(task, 0);
  return rows;
}

export function normalizeStepTitles(steps: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const raw of steps) {
    const title = raw.trim().replace(/\s+/g, " ");
    const key = title.toLocaleLowerCase();
    if (!title || seen.has(key)) continue;
    seen.add(key);
    normalized.push(title);
  }
  return normalized;
}

export function newStepSuggestions(
  generated: string[],
  existing: string[],
  maximum = MAX_ACTION_STEPS,
): string[] {
  const existingKeys = new Set(normalizeStepTitles(existing).map((title) => title.toLocaleLowerCase()));
  const room = Math.max(0, maximum - existingKeys.size);
  return normalizeStepTitles(generated)
    .filter((title) => !existingKeys.has(title.toLocaleLowerCase()))
    .slice(0, room);
}
