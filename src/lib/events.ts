// Single event channel from the engine (aos-core AppEvent, tag = "type").

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { Classification, MonitoringState } from "./ipc";

export type AppEvent =
  | { type: "ACTIVITY_CHANGED"; app_name: string; window_title: string; classification: Classification }
  | { type: "USER_IDLE" }
  | { type: "USER_ACTIVE" }
  | { type: "FOCUS_STARTED"; commitment_id: number }
  | { type: "FOCUS_ENDED"; commitment_id: number }
  | { type: "COMMITMENT_CHANGED"; commitment_id: number | null }
  | { type: "DISTRACTION_WARNING"; distracted_secs: number }
  | { type: "DISTRACTION_DETECTED"; distracted_secs: number; app_name: string; window_title: string }
  | { type: "DISTRACTION_RESOLVED"; recovery_secs: number | null }
  | { type: "CHECKIN_DUE"; checkin_id: number }
  | { type: "CHECKIN_ANSWERED"; checkin_id: number }
  | { type: "PRIORITY_CHANGE_REQUESTED"; commitment_id: number | null }
  | { type: "BLOCKED_FLOW_REQUESTED"; commitment_id: number | null }
  | { type: "BREAK_STARTED"; ends_at: number }
  | { type: "BREAK_ENDED" }
  | { type: "TASK_COMPLETED"; task_id: number }
  | { type: "DAY_LOCKED"; plan_id: number }
  | { type: "DAY_ENDED"; plan_id: number }
  | { type: "INTERVIEW_DUE" }
  | { type: "REVIEW_DUE" }
  | { type: "MONITORING_STATUS"; state: MonitoringState }
  | { type: "SESSIONS_UPDATED" }
  | { type: "SCORES_UPDATED" };

export function onAppEvent(handler: (event: AppEvent) => void): Promise<UnlistenFn> {
  return listen<AppEvent>("app-event", (e) => handler(e.payload));
}
