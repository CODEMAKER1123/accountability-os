// Typed IPC surface. Every backend command has exactly one wrapper here;
// the types mirror the Rust row/DTO structs field-for-field.

import { invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// Shared enums (string unions on this side)
// ---------------------------------------------------------------------------

export type Classification =
  | "focused"
  | "supporting"
  | "neutral"
  | "distracted"
  | "idle"
  | "unknown";

export type ClassificationSource =
  | "manual"
  | "correction"
  | "rule"
  | "cache"
  | "ai"
  | "default";

export type TaskStatus =
  | "inbox"
  | "planned"
  | "committed"
  | "active"
  | "completed"
  | "deferred"
  | "cancelled";

export type Priority = "must" | "should" | "could";

export type MonitoringState = "active" | "paused" | "permission_required" | "demo";

export type CommitmentStatus =
  | "pending"
  | "active"
  | "completed"
  | "deferred"
  | "dropped"
  | "cancelled";

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

export interface Project {
  id: number;
  name: string;
  color: string | null;
  archived: boolean;
  created_at: number;
}

export interface Task {
  id: number;
  title: string;
  description: string;
  project_id: number | null;
  parent_task_id: number | null;
  status: TaskStatus;
  priority: Priority;
  estimated_minutes: number | null;
  due_date: string | null;
  tags: string[];
  created_at: number;
  completed_at: number | null;
}

export interface TaskInput {
  title: string;
  description?: string;
  project_id?: number | null;
  parent_task_id?: number | null;
  status?: TaskStatus;
  priority?: Priority;
  estimated_minutes?: number | null;
  due_date?: string | null;
  tags?: string[];
}

export interface DailyPlan {
  id: number;
  date: string;
  locked_at: number | null;
  ended_at: number | null;
  likely_distraction: string;
  countermeasure: string;
  most_important_when: string;
  is_day_off: boolean;
  created_at: number;
}

export interface Commitment {
  id: number;
  plan_id: number;
  task_id: number | null;
  title: string;
  done_definition: string;
  estimated_minutes: number | null;
  priority: Priority;
  rank: number;
  status: CommitmentStatus;
  started_at: number | null;
  completed_at: number | null;
  outcome_reason: string | null;
  outcome_note: string | null;
  steps: CommitmentStep[];
}

export interface CommitmentStep {
  title: string;
  completed: boolean;
}

export interface ActivitySession {
  id: number;
  local_date: string;
  started_at: number;
  ended_at: number;
  duration_seconds: number;
  application_name: string;
  process_name: string;
  window_title: string;
  browser_domain: string | null;
  browser_title: string | null;
  classification: Classification;
  classification_confidence: number | null;
  classification_source: ClassificationSource;
  classification_reason: string | null;
  related_commitment_id: number | null;
  is_idle: boolean;
  pending_ai: boolean;
}

export interface FocusSession {
  id: number;
  commitment_id: number;
  started_at: number;
  ended_at: number | null;
  outcome: string | null;
}

export interface ClassifyOutcome {
  classification: Classification;
  confidence: number;
  source: ClassificationSource;
  reason: string;
}

export interface CurrentActivity {
  app_name: string;
  process_name: string;
  window_title: string;
  browser_domain: string | null;
  is_idle: boolean;
  outcome: ClassifyOutcome;
  since: number;
}

export interface ActiveCommitment {
  id: number;
  title: string;
  done_definition: string;
  project_id: number | null;
}

export interface DayTotals {
  focused_secs: number;
  supporting_secs: number;
  neutral_secs: number;
  distracted_secs: number;
  idle_secs: number;
  unknown_secs: number;
}

export interface DailyScore {
  date: string;
  total: number | null;
  completion: number | null;
  alignment: number | null;
  focus_quality: number | null;
  planning_accuracy: number | null;
  focused_secs: number;
  supporting_secs: number;
  neutral_secs: number;
  distracted_secs: number;
  idle_secs: number;
  unknown_secs: number;
  context_switches: number;
}

export interface BreakInfo {
  started_at: number;
  ends_at: number;
}

export interface CommitmentProgress {
  commitment_id: number;
  focused_secs: number;
}

export interface WorkHoursSummary {
  start_min: number;
  end_min: number;
  in_work_hours: boolean;
  elapsed_secs: number;
  tracked_secs: number;
  gap_secs: number;
  productivity: number | null;
  totals: DayTotals;
}

export interface TodaySnapshot {
  date: string;
  plan: DailyPlan | null;
  commitments: Commitment[];
  active_commitment: ActiveCommitment | null;
  focus_session: FocusSession | null;
  totals: DayTotals;
  score: DailyScore;
  current: CurrentActivity | null;
  monitoring_state: MonitoringState;
  monitoring_message: string | null;
  next_checkin_at: number;
  current_break: BreakInfo | null;
  distracted_secs: number;
  warned: boolean;
  commitment_progress: CommitmentProgress[];
  extension_connected: boolean;
  work_hours: WorkHoursSummary;
}

export interface Settings {
  work_start_min: number;
  work_end_min: number;
  interview_time_min: number;
  review_time_min: number;
  checkin_cadence_min: number;
  distraction_warn_secs: number;
  distraction_intervene_secs: number;
  strict_mode: boolean;
  idle_threshold_secs: number;
  launch_at_startup: boolean;
  start_minimized: boolean;
  widget_enabled: boolean;
  widget_always_on_top: boolean;
  monitoring_consent: boolean;
  browser_monitoring_enabled: boolean;
  activity_retention_days: number;
  excluded_apps: string[];
  excluded_domains: string[];
  private_apps: string[];
  demo_mode: boolean;
  ai_classification_enabled: boolean;
  ai_coaching_enabled: boolean;
  ai_base_url: string;
  ai_classify_model: string;
  ai_coach_model: string;
  extension_port: number;
  extension_token: string;
  onboarding_completed: boolean;
}

export interface Interruption {
  id: number;
  kind: string;
  commitment_id: number | null;
  app_name: string;
  process_name: string;
  browser_domain: string | null;
  window_title: string;
  distracted_secs: number;
  started_at: number;
  acknowledged_at: number | null;
  response: string | null;
  response_note: string | null;
  returned_at: number | null;
  recovery_secs: number | null;
}

export interface Checkin {
  id: number;
  due_at: number;
  shown_at: number | null;
  commitment_id: number | null;
  window_stats: {
    focused_secs?: number;
    supporting_secs?: number;
    neutral_secs?: number;
    distracted_secs?: number;
    idle_secs?: number;
    unknown_secs?: number;
    window_start?: number;
  } | null;
  response: string | null;
  response_note: string | null;
}

export type PendingPrompt =
  | { kind: "intervention"; interruption: Interruption; commitment_title: string | null }
  | { kind: "checkin"; checkin: Checkin; commitment_title: string | null; cadence_min: number }
  | { kind: "break_over"; commitment_title: string | null }
  | { kind: "none" };

export interface DomainRule {
  id: number;
  domain: string;
  classification: Classification;
  project_id: number | null;
  commitment_id: number | null;
  only_in_focus: boolean;
  is_default: boolean;
}

export interface AppRule {
  id: number;
  process_name: string;
  classification: Classification;
  project_id: number | null;
  commitment_id: number | null;
  only_in_focus: boolean;
}

export interface MonitoringStatus {
  state: MonitoringState;
  message: string | null;
  extension_connected: boolean;
  current: CurrentActivity | null;
  distracted_secs: number;
  warned: boolean;
}

export interface HourStat {
  hour: number;
  focused_secs: number;
  distracted_secs: number;
  total_secs: number;
}

export interface DistractionStats {
  total_secs: number;
  session_count: number;
  longest_secs: number;
  top_sources: [string, number][];
  avg_recovery_secs: number | null;
}

export interface PatternsReport {
  days: number;
  hourly: HourStat[];
  top_distractions: [string, number][];
  top_apps: [string, number][];
  deep_work_blocks: number;
  longest_deep_block_secs: number;
  context_switches: number;
  estimation_bias: number | null;
  completion_before_noon: number | null;
  completion_after_noon: number | null;
  avg_recovery_secs: number | null;
  distraction_stats: DistractionStats;
}

export interface Insight {
  id: number;
  period: string;
  metric: string;
  text: string;
  source: string;
  created_at: number;
}

export interface ReviewData {
  plan: DailyPlan;
  commitments: Commitment[];
  score: DailyScore;
  commitment_progress: CommitmentProgress[];
  ai_summary: string | null;
  already_reviewed: boolean;
}

export interface MorningCoach {
  text: string;
  source: "ai" | "deterministic";
  avg_completed_per_day: number | null;
  estimation_bias: number | null;
  completion_before_noon: number | null;
  completion_after_noon: number | null;
}

export interface CommitmentInput {
  task_id?: number | null;
  title: string;
  done_definition: string;
  estimated_minutes?: number | null;
  priority?: Priority;
  steps?: string[];
}

export type BreakdownDetail = "simple" | "standard" | "detailed";

export interface LockDayInput {
  date: string;
  commitments: CommitmentInput[];
  likely_distraction?: string;
  countermeasure?: string;
  most_important_when?: string;
  interview_answers?: unknown;
}

export interface ReviseCommitmentInput extends CommitmentInput {
  id: number | null;
}

export interface ReviseDayInput {
  date: string;
  commitments: ReviseCommitmentInput[];
  likely_distraction?: string;
  countermeasure?: string;
  most_important_when?: string;
  interview_answers?: unknown;
}

export interface TodayPlan {
  plan: DailyPlan | null;
  commitments: Commitment[];
}

export interface ExtensionInfo {
  port: number;
  token: string;
  connected: boolean;
  last_report_at: number | null;
}

export interface TimelineData {
  date: string;
  day_start_ts: number;
  day_end_ts: number;
  sessions: ActivitySession[];
}

export interface IpcError {
  code: string;
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    return String((e as IpcError).message);
  }
  return String(e);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export const api = {
  // tasks
  listTasks: (status?: TaskStatus | null, search?: string | null) =>
    invoke<Task[]>("list_tasks", { status: status ?? null, search: search ?? null }),
  createTask: (input: TaskInput) => invoke<Task>("create_task", { input }),
  createTaskSteps: (id: number, steps: string[]) =>
    invoke<Task[]>("create_task_steps", { id, steps }),
  updateTask: (id: number, input: TaskInput) => invoke<Task>("update_task", { id, input }),
  setTaskStatus: (id: number, status: TaskStatus) =>
    invoke<Task>("set_task_status", { id, status }),
  deleteTask: (id: number) => invoke<void>("delete_task", { id }),
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string, color?: string) =>
    invoke<Project>("create_project", { name, color: color ?? null }),
  archiveProject: (id: number) => invoke<void>("archive_project", { id }),

  // plan / interview
  getTodayPlan: () => invoke<TodayPlan>("get_today_plan"),
  getPlanForDate: (date: string) => invoke<TodayPlan>("get_plan_for_date", { date }),
  lockDay: (input: LockDayInput) => invoke<TodayPlan>("lock_day", { input }),
  reviseDay: (input: ReviseDayInput) => invoke<TodayPlan>("revise_day", { input }),
  markDayOff: (date?: string) => invoke<DailyPlan>("mark_day_off", { date: date ?? null }),
  snoozeInterview: (minutes?: number) =>
    invoke<{ allowed: boolean; message: string | null; snoozed_until: number | null }>(
      "snooze_interview",
      { minutes: minutes ?? null },
    ),
  commitmentLimitCheck: (selected: number) =>
    invoke<string | null>("commitment_limit_check", { selected }),
  startTask: (taskId: number) => invoke<Commitment>("start_task", { taskId }),
  setCommitmentStepCompleted: (commitmentId: number, stepIndex: number, completed: boolean) =>
    invoke<Commitment>("set_commitment_step_completed", {
      commitmentId,
      stepIndex,
      completed,
    }),
  addCommitmentSteps: (commitmentId: number, steps: string[]) =>
    invoke<Commitment>("add_commitment_steps", { commitmentId, steps }),

  // focus
  startCommitment: (commitmentId: number) =>
    invoke<Commitment>("start_commitment", { commitmentId }),
  pauseFocus: () => invoke<void>("pause_focus"),
  completeCommitment: (commitmentId: number) =>
    invoke<Commitment>("complete_commitment", { commitmentId }),
  blockCommitment: (input: {
    commitment_id: number;
    blocker_kind: string;
    note?: string | null;
    next_action?: string | null;
  }) => invoke<number | null>("block_commitment", { input }),
  switchCommitment: (input: {
    to_commitment_id: number | null;
    to_task_id?: number | null;
    from_commitment_id: number | null;
    reason: string;
    original_disposition?: string;
  }) => invoke<Commitment | null>("switch_commitment", { input }),

  // activity
  getActivityForDate: (date?: string) =>
    invoke<ActivitySession[]>("get_activity_for_date", { date: date ?? null }),
  getTimeline: (date?: string) => invoke<TimelineData>("get_timeline", { date: date ?? null }),
  searchActivity: (query: string) => invoke<ActivitySession[]>("search_activity", { query }),
  correctSession: (input: {
    session_id: number;
    new_classification: Classification;
    reason?: string | null;
    create_rule?: { kind: "domain" | "app"; project_scoped?: boolean; only_in_focus?: boolean } | null;
  }) => invoke<ActivitySession>("correct_session", { input }),
  getMonitoringStatus: () => invoke<MonitoringStatus>("get_monitoring_status"),

  // prompts / breaks
  getPendingPrompt: () => invoke<PendingPrompt>("get_pending_prompt"),
  respondIntervention: (id: number, response: string, note?: string) =>
    invoke<void>("respond_intervention", { id, response, note: note ?? null }),
  respondCheckin: (checkinId: number, response: string, note?: string) =>
    invoke<void>("respond_checkin", { checkinId, response, note: note ?? null }),
  startBreak: (minutes: number) => invoke<{ id: number; planned_end_at: number }>("start_break", { minutes }),
  endBreakNow: () => invoke<void>("end_break_now"),
  acknowledgeBreakOver: () => invoke<void>("acknowledge_break_over"),

  // scores / review / analytics
  getTodaySnapshot: () => invoke<TodaySnapshot>("get_today_snapshot"),
  getDayScore: (date?: string) => invoke<DailyScore>("get_day_score", { date: date ?? null }),
  getScorecard: (fromDate: string, toDate: string) =>
    invoke<DailyScore[]>("get_scorecard", { fromDate, toDate }),
  getReviewData: (date?: string) => invoke<ReviewData>("get_review_data", { date: date ?? null }),
  submitReview: (input: {
    date?: string | null;
    items: { commitment_id: number; completed: boolean; reason?: string | null; note?: string | null }[];
  }) => invoke<DailyScore>("submit_review", { input }),
  delayReview: (minutes?: number) => invoke<void>("delay_review", { minutes: minutes ?? null }),
  getPatterns: (days: number) => invoke<PatternsReport>("get_patterns", { days }),
  getInsights: (days: number) => invoke<Insight[]>("get_insights", { days }),

  // settings / data / windows
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  pauseMonitoring: () => invoke<void>("pause_monitoring"),
  resumeMonitoring: () => invoke<void>("resume_monitoring"),
  grantMonitoringConsent: () => invoke<Settings>("grant_monitoring_consent"),
  setDemoMode: (enabled: boolean) => invoke<Settings>("set_demo_mode", { enabled }),
  seedDemoData: () => invoke<string>("seed_demo_data"),
  listRules: () => invoke<{ domain_rules: DomainRule[]; app_rules: AppRule[] }>("list_rules"),
  addDomainRule: (domain: string, classification: Classification, onlyInFocus: boolean) =>
    invoke<void>("add_domain_rule", { domain, classification, onlyInFocus }),
  addAppRule: (processName: string, classification: Classification, onlyInFocus: boolean) =>
    invoke<void>("add_app_rule", { processName, classification, onlyInFocus }),
  deleteRule: (kind: "domain" | "app", id: number) => invoke<void>("delete_rule", { kind, id }),
  deleteActivity: (scope: "today" | "range" | "all", fromDate?: string, toDate?: string) =>
    invoke<number>("delete_activity", {
      scope,
      fromDate: fromDate ?? null,
      toDate: toDate ?? null,
    }),
  exportData: (path: string) => invoke<string>("export_data", { path }),
  getExtensionInfo: () => invoke<ExtensionInfo>("get_extension_info"),
  setWidgetVisible: (visible: boolean) => invoke<void>("set_widget_visible", { visible }),
  openQuickCapture: () => invoke<void>("open_quick_capture"),
  closeWindow: (label: string) => invoke<void>("close_window", { label }),
  showMainWindow: () => invoke<void>("show_main_window"),
  triggerReviewNow: () => invoke<void>("trigger_review_now"),

  // ai
  setAiKey: (key: string) => invoke<boolean>("set_ai_key", { key }),
  hasAiKey: () => invoke<boolean>("has_ai_key"),
  testAiConnection: () => invoke<string>("test_ai_connection"),
  breakDownGoal: (goal: string, detail: BreakdownDetail) =>
    invoke<{ steps: string[] }>("break_down_goal", { goal, detail }),
  getMorningCoach: (proposed: string[]) => invoke<MorningCoach>("get_morning_coach", { proposed }),
  generateDailyAiReview: (date?: string) =>
    invoke<string>("generate_daily_ai_review", { date: date ?? null }),
  generateAiInsights: (days: number) => invoke<Insight[]>("generate_ai_insights", { days }),
};
