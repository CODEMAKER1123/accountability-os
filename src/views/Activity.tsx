// Activity — what actually happened (spec §4 Activity, §43 search, §44
// timeline), with inline reclassification (spec §42).

import { useCallback, useEffect, useState } from "react";

import Timeline from "@/components/Timeline";
import { ClassBadge, EmptyState, CLASS_LABEL } from "@/components/shared";
import {
  api,
  errorMessage,
  type ActivitySession,
  type Classification,
  type TimelineData,
} from "@/lib/ipc";
import { onAppEvent } from "@/lib/events";
import { addDaysISO, fmtDate, fmtDuration, fmtTime, todayISO } from "@/lib/time";

export default function Activity() {
  const [date, setDate] = useState(todayISO());
  const [timeline, setTimeline] = useState<TimelineData | null>(null);
  const [search, setSearch] = useState("");
  const [searchResults, setSearchResults] = useState<ActivitySession[] | null>(null);
  const [correcting, setCorrecting] = useState<ActivitySession | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setTimeline(await api.getTimeline(date));
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [date]);

  useEffect(() => {
    void reload();
    const unlisten = onAppEvent((e) => {
      if (e.type === "SESSIONS_UPDATED") void reload();
    });
    return () => void unlisten.then((fn) => fn());
  }, [reload]);

  useEffect(() => {
    if (search.trim().length < 2) {
      setSearchResults(null);
      return;
    }
    const t = setTimeout(() => {
      void api.searchActivity(search).then(setSearchResults).catch(() => setSearchResults([]));
    }, 250);
    return () => clearTimeout(t);
  }, [search]);

  const sessions = searchResults ?? timeline?.sessions ?? [];

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-lg font-semibold text-ink-50">Activity</h1>
        <div className="flex items-center gap-2">
          <input
            className="input w-64"
            placeholder='Search apps, titles, domains… e.g. "Outlook"'
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          {!searchResults && (
            <>
              <button className="btn px-2" onClick={() => setDate(addDaysISO(date, -1))}>
                ‹
              </button>
              <span className="w-28 text-center text-xs text-ink-200">{fmtDate(date)}</span>
              <button
                className="btn px-2"
                onClick={() => setDate(addDaysISO(date, 1))}
                disabled={date >= todayISO()}
              >
                ›
              </button>
            </>
          )}
        </div>
      </div>

      {error && <p className="text-xs text-distracted">{error}</p>}

      {!searchResults && timeline && timeline.sessions.length > 0 && (
        <Timeline data={timeline} onSelect={setCorrecting} />
      )}

      {sessions.length === 0 ? (
        <EmptyState
          title={searchResults ? "No matches" : "No activity recorded for this day"}
          hint={
            searchResults
              ? undefined
              : "Sessions appear here as the monitor aggregates your foreground activity."
          }
        />
      ) : (
        <div className="divide-y divide-ink-800 rounded-lg border border-ink-700 bg-ink-900">
          {sessions.map((s) => (
            <button
              key={s.id}
              className="flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-ink-850"
              onClick={() => setCorrecting(s)}
              title="Click to reclassify"
            >
              <span className="w-24 shrink-0 font-mono text-2xs text-ink-500">
                {fmtTime(s.started_at)}–{fmtTime(s.ended_at)}
              </span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] text-ink-100">
                  {s.is_idle ? "Desktop idle" : s.application_name}
                  {s.browser_domain && <span className="ml-2 text-xs text-ink-400">{s.browser_domain}</span>}
                </p>
                <p className="truncate text-2xs text-ink-500">{s.window_title || "—"}</p>
              </div>
              <span className="shrink-0 font-mono text-2xs text-ink-400">
                {fmtDuration(s.duration_seconds)}
              </span>
              <span className="w-28 shrink-0 text-right">
                <ClassBadge classification={s.classification} />
              </span>
            </button>
          ))}
        </div>
      )}

      {correcting && (
        <CorrectionDialog
          session={correcting}
          onClose={() => setCorrecting(null)}
          onSaved={async () => {
            setCorrecting(null);
            await reload();
            if (searchResults) setSearchResults(null);
          }}
        />
      )}
    </div>
  );
}

function CorrectionDialog({
  session,
  onClose,
  onSaved,
}: {
  session: ActivitySession;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [classification, setClassification] = useState<Classification>(
    session.classification === "idle" || session.classification === "unknown"
      ? "neutral"
      : session.classification,
  );
  const [reason, setReason] = useState("");
  const [ruleKind, setRuleKind] = useState<"none" | "domain" | "app">("none");
  const [ruleScoped, setRuleScoped] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.correctSession({
        session_id: session.id,
        new_classification: classification,
        reason: reason || null,
        create_rule:
          ruleKind === "none"
            ? null
            : { kind: ruleKind, project_scoped: ruleScoped, only_in_focus: false },
      });
      await onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div
        className="w-full max-w-md rounded-xl border border-ink-600 bg-ink-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="section-title mb-1">Reclassify activity</p>
        <p className="truncate text-sm font-medium text-ink-50">
          {session.is_idle ? "Desktop idle" : session.application_name}
          {session.browser_domain && ` · ${session.browser_domain}`}
        </p>
        <p className="truncate text-xs text-ink-400">{session.window_title}</p>
        <p className="mt-1 text-2xs text-ink-500">
          {fmtTime(session.started_at)}–{fmtTime(session.ended_at)} ·{" "}
          {fmtDuration(session.duration_seconds)} · currently {CLASS_LABEL[session.classification]}
          {session.classification_reason && ` (${session.classification_reason})`}
        </p>

        <div className="mt-4 grid grid-cols-4 gap-1.5">
          {(["focused", "supporting", "neutral", "distracted"] as Classification[]).map((c) => (
            <button
              key={c}
              className={`rounded-md border px-2 py-1.5 text-xs capitalize ${
                classification === c
                  ? "border-accent bg-accent/10 text-ink-50"
                  : "border-ink-600 text-ink-300 hover:border-ink-500"
              }`}
              onClick={() => setClassification(c)}
            >
              {c}
            </button>
          ))}
        </div>

        <input
          className="input mt-3"
          placeholder="Reason (optional) — e.g. training video for the project"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />

        <div className="mt-3 space-y-1.5 text-xs text-ink-300">
          <label className="flex items-center gap-2">
            <input
              type="radio"
              name="rule"
              checked={ruleKind === "none"}
              onChange={() => setRuleKind("none")}
              className="accent-[#5b8def]"
            />
            Just this session
          </label>
          {session.browser_domain && (
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name="rule"
                checked={ruleKind === "domain"}
                onChange={() => setRuleKind("domain")}
                className="accent-[#5b8def]"
              />
              Always classify {session.browser_domain} as {classification}
            </label>
          )}
          {!session.is_idle &&
            session.process_name.trim() !== "" &&
            session.process_name !== "__private__" && (
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  name="rule"
                  checked={ruleKind === "app"}
                  onChange={() => setRuleKind("app")}
                  className="accent-[#5b8def]"
                />
                Always classify {session.application_name} as {classification}
              </label>
            )}
          {ruleKind !== "none" && (
            <label className="ml-5 flex items-center gap-2 text-ink-400">
              <input
                type="checkbox"
                checked={ruleScoped}
                onChange={(e) => setRuleScoped(e.target.checked)}
                className="accent-[#5b8def]"
              />
              Only while working on the current project
            </label>
          )}
        </div>

        {error && <p className="mt-2 text-xs text-distracted">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={() => void save()} disabled={busy}>
            Save correction
          </button>
        </div>
      </div>
    </div>
  );
}
