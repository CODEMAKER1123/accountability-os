// Always-on-top mini focus widget (spec §25).

import { useEffect, useState } from "react";

import { api, type TodaySnapshot } from "@/lib/ipc";
import { fmtClockDuration, fmtDuration, fmtPct } from "@/lib/time";

export default function WidgetWindow() {
  const [snap, setSnap] = useState<TodaySnapshot | null>(null);
  const [, force] = useState(0);

  useEffect(() => {
    const load = () => void api.getTodaySnapshot().then(setSnap).catch(() => undefined);
    load();
    const poll = setInterval(load, 5000);
    const tick = setInterval(() => force((n) => n + 1), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  }, []);

  const now = Math.floor(Date.now() / 1000);
  const active = snap?.active_commitment ?? null;
  const elapsed = snap?.focus_session ? now - snap.focus_session.started_at : 0;
  const onBreak = snap?.current_break && snap.current_break.ends_at > now;

  return (
    <div
      className="flex h-screen select-none flex-col border border-ink-600 bg-ink-950 p-3"
      data-tauri-drag-region
    >
      <div className="flex items-center justify-between" data-tauri-drag-region>
        <span
          className={`text-2xs font-bold uppercase tracking-widest ${
            onBreak ? "text-accent" : active ? "text-focus" : "text-ink-500"
          }`}
          data-tauri-drag-region
        >
          {onBreak ? "Break" : active ? "Active" : "No commitment"}
        </span>
        <button
          className="text-ink-600 hover:text-ink-300"
          title="Close widget"
          onClick={() => void api.setWidgetVisible(false)}
        >
          ✕
        </button>
      </div>

      {onBreak && snap?.current_break ? (
        <div className="mt-1 flex flex-1 flex-col" data-tauri-drag-region>
          <p className="font-mono text-2xl text-ink-50">
            {fmtClockDuration(snap.current_break.ends_at - now)}
          </p>
          <p className="text-2xs text-ink-500">until break ends</p>
        </div>
      ) : active ? (
        <div className="mt-1 flex flex-1 flex-col" data-tauri-drag-region>
          <p className="truncate text-[13px] font-medium text-ink-50" title={active.title}>
            {active.title}
          </p>
          <p className="mt-0.5 font-mono text-2xl tabular-nums text-ink-50">
            {fmtClockDuration(elapsed)}
          </p>
          <p className="text-2xs text-ink-400">
            {fmtPct(snap?.score.alignment ?? null)} aligned
          </p>
          <p className="mt-auto text-2xs text-ink-500">
            <span className="text-focus">Focused {fmtDuration(snap?.totals.focused_secs ?? 0)}</span>
            {" · "}
            <span className="text-distracted">
              Distracted {fmtDuration(snap?.totals.distracted_secs ?? 0)}
            </span>
          </p>
          <div className="mt-1.5 flex gap-1.5">
            <button
              className="btn flex-1 py-1 text-2xs"
              onClick={async () => {
                await api.completeCommitment(active.id);
              }}
            >
              Done
            </button>
            <button
              className="btn flex-1 py-1 text-2xs"
              onClick={async () => {
                await api.pauseFocus();
              }}
            >
              Pause
            </button>
          </div>
        </div>
      ) : (
        <div className="mt-1 flex flex-1 flex-col" data-tauri-drag-region>
          <p className="text-xs text-ink-400">Nothing active.</p>
          <button className="btn mt-auto py-1 text-2xs" onClick={() => void api.showMainWindow()}>
            Open app
          </button>
        </div>
      )}
    </div>
  );
}
