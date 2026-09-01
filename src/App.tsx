import { useEffect } from "react";

import BlockedModal from "@/components/BlockedModal";
import AppUpdater from "@/components/AppUpdater";
import BreakModal from "@/components/BreakModal";
import Interview from "@/components/Interview";
import Onboarding from "@/components/Onboarding";
import ReviewModal from "@/components/ReviewModal";
import SwitchModal from "@/components/SwitchModal";
import { ErrorBanner, MonitoringBadge, ToastHost } from "@/components/shared";
import { onAppEvent } from "@/lib/events";
import { useStore, type View } from "@/lib/store";
import { useTheme } from "@/lib/theme";
import Activity from "@/views/Activity";
import Plan from "@/views/Plan";
import Scorecard from "@/views/Scorecard";
import Settings from "@/views/Settings";
import Tasks from "@/views/Tasks";
import Today from "@/views/Today";

const NAV: { id: View; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "tasks", label: "Tasks" },
  { id: "plan", label: "Daily Plan" },
  { id: "activity", label: "Activity" },
  { id: "scorecard", label: "Scorecard" },
  { id: "settings", label: "Settings" },
];

export default function App() {
  const {
    view,
    setView,
    modal,
    setModal,
    snapshot,
    snapshotError,
    refreshSnapshot,
    settings,
    loadSettings,
  } = useStore();
  const { theme, setTheme } = useTheme();

  useEffect(() => {
    void refreshSnapshot();
    void loadSettings().then((s) => {
      if (!s.onboarding_completed) setModal({ kind: "onboarding" });
    });
    const interval = setInterval(() => void refreshSnapshot(), 5000);
    const unlisten = onAppEvent((e) => {
      switch (e.type) {
        case "INTERVIEW_DUE":
          setModal({ kind: "interview" });
          break;
        case "REVIEW_DUE":
          setModal({ kind: "review" });
          break;
        case "PRIORITY_CHANGE_REQUESTED":
          setView("today");
          setModal({ kind: "switch", fromCommitmentId: e.commitment_id });
          break;
        case "BLOCKED_FLOW_REQUESTED":
          setView("today");
          if (e.commitment_id != null) {
            setModal({ kind: "blocked", commitmentId: e.commitment_id });
          }
          break;
        case "SESSIONS_UPDATED":
        case "SCORES_UPDATED":
        case "COMMITMENT_CHANGED":
        case "FOCUS_STARTED":
        case "FOCUS_ENDED":
        case "BREAK_STARTED":
        case "BREAK_ENDED":
        case "DAY_LOCKED":
        case "DAY_ENDED":
        case "MONITORING_STATUS":
        case "DISTRACTION_WARNING":
        case "DISTRACTION_RESOLVED":
          void refreshSnapshot();
          break;
        default:
          break;
      }
    });
    return () => {
      clearInterval(interval);
      void unlisten.then((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-full">
      {/* Sidebar */}
      <aside className="flex w-48 shrink-0 flex-col border-r border-ink-700 bg-ink-900">
        <div className="flex items-center gap-2 px-4 pb-2 pt-4">
          <div className="flex h-6 w-6 items-center justify-center rounded-md border border-ink-600 bg-ink-800">
            <div className="h-2 w-2 rounded-full bg-focus" />
          </div>
          <span className="text-[13px] font-semibold tracking-tight text-ink-50">
            Accountability OS
          </span>
        </div>
        <nav className="mt-2 flex flex-col gap-0.5 px-2">
          {NAV.map((item) => (
            <button
              key={item.id}
              onClick={() => setView(item.id)}
              aria-current={view === item.id ? "page" : undefined}
              className={`rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors ${
                view === item.id
                  ? "bg-ink-700 font-medium text-ink-50"
                  : "text-ink-300 hover:bg-ink-800 hover:text-ink-100"
              }`}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="mt-auto space-y-2 border-t border-ink-700 p-3">
          <button
            className="btn btn-ghost w-full justify-start px-2 py-1 text-xs"
            aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          >
            <span aria-hidden="true">{theme === "dark" ? "☀" : "☾"}</span>
            {theme === "dark" ? "Light mode" : "Dark mode"}
          </button>
          <AppUpdater />
          {snapshot && (
            <MonitoringBadge
              state={snapshot.monitoring_state}
              message={snapshot.monitoring_message}
            />
          )}
          {snapshot?.warned && (
            <p className="text-2xs font-medium text-warn">Drifting off task</p>
          )}
        </div>
      </aside>

      {/* Main */}
      <main className="min-w-0 flex-1 overflow-y-auto">
        {snapshotError && (
          <div className="mx-auto max-w-5xl px-6 pt-4">
            <ErrorBanner
              message={`Accountability OS could not refresh its local data: ${snapshotError}`}
              onRetry={() => void refreshSnapshot()}
            />
          </div>
        )}
        {view === "today" && <Today />}
        {view === "tasks" && <Tasks />}
        {view === "plan" && <Plan />}
        {view === "activity" && <Activity />}
        {view === "scorecard" && <Scorecard />}
        {view === "settings" && <Settings />}
      </main>

      {/* Modals */}
      {modal?.kind === "onboarding" && <Onboarding />}
      {modal?.kind === "interview" && <Interview mode={modal.mode ?? "new"} />}
      {modal?.kind === "review" && <ReviewModal />}
      {modal?.kind === "switch" && (
        <SwitchModal
          fromCommitmentId={modal.fromCommitmentId}
          toCommitmentId={modal.toCommitmentId ?? null}
          toTaskId={modal.toTaskId ?? null}
          toTaskTitle={modal.toTaskTitle}
        />
      )}
      {modal?.kind === "blocked" && <BlockedModal commitmentId={modal.commitmentId} />}
      {modal?.kind === "break" && <BreakModal />}
      {settings && !settings.monitoring_consent && settings.onboarding_completed && (
        <ConsentBanner />
      )}
      <ToastHost />
    </div>
  );
}

/// Shown if consent was somehow revoked post-onboarding (spec §41: never
/// silently fail).
function ConsentBanner() {
  const { loadSettings, refreshSnapshot } = useStore();
  return (
    <div className="fixed inset-x-0 bottom-0 z-40 flex items-center justify-between gap-4 border-t border-warn/40 bg-ink-900 px-4 py-2.5">
      <p className="text-xs text-ink-200">
        Desktop monitoring is off until you allow it. Activity stays in the local database on this
        machine.
      </p>
      <button
        className="btn btn-primary shrink-0"
        onClick={async () => {
          const { api } = await import("@/lib/ipc");
          await api.grantMonitoringConsent();
          await loadSettings();
          await refreshSnapshot();
        }}
      >
        Enable monitoring
      </button>
    </div>
  );
}
