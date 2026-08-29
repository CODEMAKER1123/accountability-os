import { describe, expect, it } from "vitest";

import type { Task } from "./ipc";
import {
  flattenTaskHierarchy,
  newStepSuggestions,
  normalizeStepTitles,
} from "./taskBreakdown";

function task(id: number, title: string, parent_task_id: number | null = null): Task {
  return {
    id,
    title,
    description: "",
    project_id: null,
    parent_task_id,
    status: "planned",
    priority: "should",
    estimated_minutes: null,
    due_date: null,
    tags: [],
    created_at: id,
    completed_at: null,
  };
}

describe("task breakdown helpers", () => {
  it("places steps beneath their parent without hiding filtered orphans", () => {
    const parent = task(1, "Publish the report");
    const child = task(2, "Proofread the report", 1);
    const grandchild = task(3, "Check the totals", 2);
    const orphan = task(4, "Email the report", 99);

    expect(flattenTaskHierarchy([child, orphan, parent, grandchild])).toEqual([
      { task: orphan, depth: 0, directChildren: [] },
      { task: parent, depth: 0, directChildren: [child] },
      { task: child, depth: 1, directChildren: [grandchild] },
      { task: grandchild, depth: 2, directChildren: [] },
    ]);
  });

  it("normalizes, deduplicates, and excludes already-saved steps", () => {
    expect(normalizeStepTitles(["  Open   the brief ", "open the brief", "", "Send it"])).toEqual([
      "Open the brief",
      "Send it",
    ]);
    expect(
      newStepSuggestions(
        ["Open the brief", "Draft copy", "Review copy", "Publish"],
        ["open the brief"],
        3,
      ),
    ).toEqual(["Draft copy", "Review copy"]);
  });
});
