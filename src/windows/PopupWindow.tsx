// The always-on-top prompt window: interventions (spec §13), periodic
// check-ins (spec §18), and break-over notices (spec §17). It asks the
// backend what is pending and renders exactly that.

import { useCallback, useEffect, useState } from "react";

import { api, type PendingPrompt } from "@/lib/ipc";
import { onAppEvent } from "@/lib/events";
import { fmtDuration, fmtTime } from "@/lib/time";

export default function PopupWindow() {
  const [prompt, setPrompt] = useState<PendingPrompt | null>(null);
  const [strict, setStrict] = useState(false);

  const reload = useCallback(async () => {
    try {
      const p = await api.getPendingPrompt();
      setPrompt(p);
      if (p.kind === "none") {
        await api.closeWindow("intervention");
      }
    } catch {
      // keep the previous prompt
    }
  }, []);

  useEffect(() => {
    void reload();
    void api.getSettings().then((s) => setStrict(s.strict_mode));
    const unlisten = onAppEvent(() => void reload());
    const t = setInterval(() => void reload(), 10000);
    return () => {
      clearInterval(t);
      void unlisten.then((fn) => fn());
    };
  }, [reload]);

  if (!prompt || prompt.kind === "none") {
    return <div className="flex h-screen items-center justify-center bg-ink-950" />;
  }

  return (
    <div className="flex h-screen flex-col bg-ink-950 p-5">
      {prompt.kind === "intervention" && <Intervention prompt={prompt} onDone={reload} strict={strict} />}
      {prompt.kind === "checkin" && <Checkin prompt={prompt} onDone={reload} strict={strict} />}
      {prompt.kind === "break_over" && <BreakOver prompt={prompt} onDone={reload} />}
    </div>
  );
}

function Intervention({
  prompt,
  onDone,
  strict,
}: {
  prompt: Extract<PendingPrompt, { kind: "intervention" }>;
  onDone: () => Promise<void>;
  strict: boolean;
}) {
  const [showBreakPicker, setShowBreakPicker] = useState(false);
  const [note, setNote] = useState("");
  const i = prompt.interruption;

  const respond = async (response: string) => {
    if (response === "planned_break") {
      // Keep the unanswered intervention alive while the duration picker is
      // visible; otherwise the backend correctly closes an idle popup before
      // the user gets a chance to choose.
      setShowBreakPicker(true);
      return;
    }
    await api.respondIntervention(i.id, response, note || undefined);
    if (response === "priority_changed" || response === "blocked") {
      // Continue the flow in the main window.
      await api.showMainWindow();
    }
    await onDone();
  };

  if (showBreakPicker) {
    return (
      <div className="space-y-3">
        <h1 className="text-lg font-bold text-ink-50">How long a break?</h1>
        <div className="grid grid-cols-4 gap-2">
          {[5, 10, 15, 30].map((m) => (
            <button
              key={m}
              className="btn"
              onClick={async () => {
                await api.startBreak(m);
                await api.respondIntervention(i.id, "planned_break", note || undefined);
                await onDone();
              }}
            >
              {m} min
            </button>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <p className="text-2xs font-bold uppercase tracking-widest text-distracted">You're off plan</p>
      <div className="mt-3 space-y-3">
        <div>
          <p className="text-2xs uppercase tracking-wider text-ink-500">You said you were working on</p>
          <p className="text-base font-semibold text-ink-50">
            {prompt.commitment_title ?? "your commitment"}
          </p>
        </div>
        <div>
          <p className="text-2xs uppercase tracking-wider text-ink-500">Current activity</p>
          <p className="text-base text-ink-100">
            {i.app_name} — {fmtDuration(i.distracted_secs)}
          </p>
          {i.window_title && <p className="truncate text-xs text-ink-400">{i.window_title}</p>}
        </div>
      </div>
      <input
        className="input mt-3"
        placeholder="Note (optional)"
        value={note}
        onChange={(e) => setNote(e.target.value)}
      />
      <div className="mt-4 grid gap-1.5">
        <button className="btn btn-primary justify-start" onClick={() => void respond("return")}>
          Return to task
        </button>
        <button className="btn justify-start" onClick={() => void respond("actually_work")}>
          This is actually work
        </button>
        <button className="btn justify-start" onClick={() => void respond("planned_break")}>
          Planned break
        </button>
        <button className="btn justify-start" onClick={() => void respond("priority_changed")}>
          Priority changed
        </button>
        <button className="btn justify-start" onClick={() => void respond("blocked")}>
          I'm blocked
        </button>
        {!strict && (
          <button
            className="btn-ghost btn justify-start text-ink-500"
            onClick={() => void respond("dismissed")}
          >
            Dismiss
          </button>
        )}
      </div>
      {strict && (
        <p className="mt-auto pt-3 text-2xs text-ink-500">
          Strict Mode: this prompt stays until you answer.
        </p>
      )}
    </div>
  );
}

function Checkin({
  prompt,
  onDone,
  strict,
}: {
  prompt: Extract<PendingPrompt, { kind: "checkin" }>;
  onDone: () => Promise<void>;
  strict: boolean;
}) {
  const [showBreakPicker, setShowBreakPicker] = useState(false);
  const stats = prompt.checkin.window_stats;

  const respond = async (response: string) => {
    if (response === "break") {
      setShowBreakPicker(true);
      return;
    }
    await api.respondCheckin(prompt.checkin.id, response);
    if (response === "priority_changed" || response === "blocked") {
      await api.showMainWindow();
    }
    await onDone();
  };

  if (showBreakPicker) {
    return (
      <div className="space-y-3">
        <h1 className="text-lg font-bold text-ink-50">How long a break?</h1>
        <div className="grid grid-cols-4 gap-2">
          {[5, 10, 15, 30].map((m) => (
            <button
              key={m}
              className="btn"
              onClick={async () => {
                await api.startBreak(m);
                await api.respondCheckin(prompt.checkin.id, "break");
                await onDone();
              }}
            >
              {m} min
            </button>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <p className="text-2xs font-bold uppercase tracking-widest text-accent">Accountability check</p>
      <div className="mt-3 space-y-3">
        <div>
          <p className="text-2xs uppercase tracking-wider text-ink-500">You committed to</p>
          <p className="text-base font-semibold text-ink-50">
            {prompt.commitment_title ?? "today's plan"}
          </p>
        </div>
        {stats && (
          <div>
            <p className="text-2xs uppercase tracking-wider text-ink-500">
              Last {prompt.cadence_min} minutes
              {stats.window_start ? ` (since ${fmtTime(stats.window_start)})` : ""}
            </p>
            <div className="mt-1 grid grid-cols-3 gap-2 text-center text-xs">
              <Cell label="Focused" v={fmtDuration(stats.focused_secs ?? 0)} cls="text-focus" />
              <Cell label="Supporting" v={fmtDuration(stats.supporting_secs ?? 0)} cls="text-supporting" />
              <Cell label="Neutral" v={fmtDuration(stats.neutral_secs ?? 0)} cls="text-neutralcat" />
              <Cell label="Distracted" v={fmtDuration(stats.distracted_secs ?? 0)} cls="text-distracted" />
              <Cell label="Idle" v={fmtDuration(stats.idle_secs ?? 0)} cls="text-ink-400" />
              <Cell label="Unknown" v={fmtDuration(stats.unknown_secs ?? 0)} cls="text-ink-400" />
            </div>
          </div>
        )}
        <p className="text-sm text-ink-100">Are you still working on the right thing?</p>
      </div>
      <div className="mt-4 grid gap-1.5">
        <button className="btn btn-primary justify-start" onClick={() => void respond("yes")}>
          Yes — keep going
        </button>
        <button className="btn justify-start" onClick={() => void respond("priority_changed")}>
          Priority changed
        </button>
        <button className="btn justify-start" onClick={() => void respond("blocked")}>
          I'm blocked
        </button>
        <button className="btn justify-start" onClick={() => void respond("break")}>
          Take a break
        </button>
      </div>
      {strict && (
        <p className="mt-auto pt-3 text-2xs text-ink-500">Strict Mode: answer to dismiss.</p>
      )}
    </div>
  );
}

function BreakOver({
  prompt,
  onDone,
}: {
  prompt: Extract<PendingPrompt, { kind: "break_over" }>;
  onDone: () => Promise<void>;
}) {
  return (
    <div className="flex h-full flex-col">
      <p className="text-2xs font-bold uppercase tracking-widest text-focus">Break is over</p>
      {prompt.commitment_title ? (
        <p className="mt-3 text-base text-ink-100">
          Return to: <span className="font-semibold text-ink-50">{prompt.commitment_title}</span>
        </p>
      ) : (
        <p className="mt-3 text-base text-ink-100">Back to it.</p>
      )}
      <div className="mt-4 grid gap-1.5">
        <button
          className="btn btn-primary justify-start"
          onClick={async () => {
            await api.acknowledgeBreakOver();
            await onDone();
          }}
        >
          Back to work
        </button>
        <button
          className="btn justify-start"
          onClick={async () => {
            await api.acknowledgeBreakOver();
            await api.startBreak(5);
            await onDone();
          }}
        >
          5 more minutes
        </button>
      </div>
    </div>
  );
}

function Cell({ label, v, cls }: { label: string; v: string; cls: string }) {
  return (
    <div className="rounded border border-ink-700 bg-ink-900 p-1.5">
      <p className={`font-mono ${cls}`}>{v}</p>
      <p className="text-2xs text-ink-500">{label}</p>
    </div>
  );
}
