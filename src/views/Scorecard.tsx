// Scorecard — daily/weekly/monthly performance + long-term patterns
// (spec §19–20, §23, Phase 5 analytics). The math is always shown.

import { useCallback, useEffect, useState } from "react";

import { ErrorBanner, ScoreRing } from "@/components/shared";
import {
  api,
  errorMessage,
  type DailyScore,
  type Insight,
  type PatternsReport,
} from "@/lib/ipc";
import { onAppEvent } from "@/lib/events";
import { addDaysISO, fmtDate, fmtDuration, fmtPct, fmtScore, hourLabel, todayISO } from "@/lib/time";
import { useStore } from "@/lib/store";

type Range = 7 | 30 | 90;

export default function Scorecard() {
  const [range, setRange] = useState<Range>(7);
  const [scores, setScores] = useState<DailyScore[]>([]);
  const [patterns, setPatterns] = useState<PatternsReport | null>(null);
  const [insights, setInsights] = useState<Insight[]>([]);
  const [aiBusy, setAiBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const settings = useStore((s) => s.settings);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const to = todayISO();
      const from = addDaysISO(to, -(range - 1));
      const [sc, pat, ins] = await Promise.all([
        api.getScorecard(from, to),
        api.getPatterns(range),
        api.getInsights(range),
      ]);
      setScores(sc);
      setPatterns(pat);
      setInsights(ins);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [range]);

  useEffect(() => {
    void reload();
    const unlisten = onAppEvent((e) => {
      if (e.type === "SCORES_UPDATED") void reload();
    });
    return () => void unlisten.then((fn) => fn());
  }, [reload]);

  const today = scores.find((s) => s.date === todayISO()) ?? scores[scores.length - 1];

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-6">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-ink-50">Scorecard</h1>
        <div className="flex gap-1">
          {([7, 30, 90] as Range[]).map((r) => (
            <button
              key={r}
              onClick={() => setRange(r)}
              aria-pressed={range === r}
              className={`rounded px-2.5 py-1 text-xs ${
                range === r ? "bg-ink-700 text-ink-50" : "text-ink-400 hover:text-ink-200"
              }`}
            >
              {r} days
            </button>
          ))}
        </div>
      </div>

      {error && <ErrorBanner message={error} onRetry={() => void reload()} onDismiss={() => setError(null)} />}
      {loading && scores.length === 0 && (
        <div className="card text-xs text-ink-400" role="status">
          Loading scorecard…
        </div>
      )}

      {/* Latest day breakdown — show the math (spec §20) */}
      {today && (
        <div className="card">
          <p className="section-title mb-3">
            {today.date === todayISO() ? "Today so far" : fmtDate(today.date)}
          </p>
          <div className="flex items-center gap-8">
            <ScoreRing value={today.total} size={88} label="Execution score" />
            <div className="grid flex-1 grid-cols-4 gap-3 text-center">
              <Component label="Completion" weight="40%" value={today.completion} />
              <Component label="Alignment" weight="30%" value={today.alignment} />
              <Component label="Focus quality" weight="20%" value={today.focus_quality} />
              <Component label="Planning accuracy" weight="10%" value={today.planning_accuracy} />
            </div>
          </div>
          <p className="mt-3 border-t border-ink-700 pt-2 text-2xs text-ink-500">
            Total = 0.4×completion + 0.3×alignment + 0.2×focus + 0.1×planning (missing components
            redistribute their weight). Focused {fmtDuration(today.focused_secs)} · supporting{" "}
            {fmtDuration(today.supporting_secs)} · neutral {fmtDuration(today.neutral_secs)} ·
            distracted {fmtDuration(today.distracted_secs)} · idle {fmtDuration(today.idle_secs)} ·{" "}
            {today.context_switches} context switches.
          </p>
        </div>
      )}

      {/* Score history bars */}
      <div className="card">
        <p className="section-title mb-3">Execution score history</p>
        {scores.length === 0 ? (
          <p className="text-xs text-ink-500">
            {loading
              ? "Loading score history…"
              : "No scored days yet. Scores are stored when you run the end-of-day review."}
          </p>
        ) : (
          <div className="flex h-28 items-end gap-1">
            {scores.map((s) => (
              <div key={s.date} className="group relative flex-1">
                <div
                  className={`w-full rounded-t ${
                    (s.total ?? 0) >= 75 ? "bg-focus" : (s.total ?? 0) >= 50 ? "bg-warn" : "bg-distracted"
                  } opacity-70 group-hover:opacity-100`}
                  style={{ height: `${Math.max(3, s.total ?? 0)}%` }}
                  title={`${fmtDate(s.date)} — score ${fmtScore(s.total)} · alignment ${fmtPct(
                    s.alignment,
                  )} · focused ${fmtDuration(s.focused_secs)}`}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      {patterns && (
        <>
          {/* Hourly profile */}
          <div className="card">
            <p className="section-title mb-3">Focus by hour of day</p>
            <div className="flex h-24 items-end gap-0.5">
              {patterns.hourly.map((h) => {
                const max = Math.max(1, ...patterns.hourly.map((x) => x.total_secs));
                return (
                  <div
                    key={h.hour}
                    className="flex flex-1 flex-col justify-end gap-px"
                    title={`${hourLabel(h.hour)} — focused ${fmtDuration(h.focused_secs)}, distracted ${fmtDuration(h.distracted_secs)}`}
                  >
                    <div
                      className="w-full rounded-t-sm bg-focus/80"
                      style={{ height: `${(h.focused_secs / max) * 88}px` }}
                    />
                    <div
                      className="w-full bg-distracted/70"
                      style={{ height: `${(h.distracted_secs / max) * 88}px` }}
                    />
                  </div>
                );
              })}
            </div>
            <div className="mt-1 flex justify-between text-2xs text-ink-500">
              <span>12 AM</span>
              <span>6 AM</span>
              <span>12 PM</span>
              <span>6 PM</span>
              <span>12 AM</span>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            {/* Distraction analysis */}
            <div className="card">
              <p className="section-title mb-3">Distraction analysis</p>
              <dl className="space-y-1.5 text-xs">
                <Row k="Total distracted" v={fmtDuration(patterns.distraction_stats.total_secs)} />
                <Row k="Distraction sessions" v={String(patterns.distraction_stats.session_count)} />
                <Row k="Longest distraction" v={fmtDuration(patterns.distraction_stats.longest_secs)} />
                <Row
                  k="Avg recovery time"
                  v={
                    patterns.avg_recovery_secs != null
                      ? fmtDuration(patterns.avg_recovery_secs)
                      : "—"
                  }
                />
              </dl>
              {patterns.top_distractions.length > 0 && (
                <div className="mt-3 border-t border-ink-700 pt-2">
                  <p className="mb-1 text-2xs uppercase tracking-wider text-ink-500">Top sources</p>
                  {patterns.top_distractions.map(([src, secs]) => (
                    <div key={src} className="flex justify-between text-xs">
                      <span className="truncate text-ink-200">{src}</span>
                      <span className="font-mono text-distracted">{fmtDuration(secs)}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Work patterns */}
            <div className="card">
              <p className="section-title mb-3">Work patterns</p>
              <dl className="space-y-1.5 text-xs">
                <Row k="Deep work blocks (≥25m)" v={String(patterns.deep_work_blocks)} />
                <Row k="Longest deep block" v={fmtDuration(patterns.longest_deep_block_secs)} />
                <Row k="Context switches" v={String(patterns.context_switches)} />
                <Row
                  k="Estimation bias"
                  v={
                    patterns.estimation_bias != null
                      ? `${patterns.estimation_bias > 1 ? "+" : ""}${Math.round(
                          (patterns.estimation_bias - 1) * 100,
                        )}% vs estimates`
                      : "—"
                  }
                />
                <Row k="Completion (started before noon)" v={fmtPct(patterns.completion_before_noon)} />
                <Row k="Completion (started after noon)" v={fmtPct(patterns.completion_after_noon)} />
              </dl>
              {patterns.top_apps.length > 0 && (
                <div className="mt-3 border-t border-ink-700 pt-2">
                  <p className="mb-1 text-2xs uppercase tracking-wider text-ink-500">Where time went</p>
                  {patterns.top_apps.slice(0, 5).map(([src, secs]) => (
                    <div key={src} className="flex justify-between text-xs">
                      <span className="truncate text-ink-200">{src}</span>
                      <span className="font-mono text-ink-400">{fmtDuration(secs)}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </>
      )}

      {/* Insights */}
      <div className="card">
        <div className="mb-3 flex items-center justify-between">
          <p className="section-title">Insights</p>
          {settings?.ai_coaching_enabled && (
            <button
              className="btn py-1 text-xs"
              disabled={aiBusy}
              onClick={async () => {
                setAiBusy(true);
                try {
                  setInsights(await api.generateAiInsights(range));
                } catch (e) {
                  setError(errorMessage(e));
                } finally {
                  setAiBusy(false);
                }
              }}
            >
              {aiBusy ? "Analyzing…" : "AI narrative"}
            </button>
          )}
        </div>
        {insights.length === 0 ? (
          <p className="text-xs text-ink-500">
            {loading
              ? "Loading insights…"
              : "Not enough history yet. Insights appear after a few days of monitored, planned work."}
          </p>
        ) : (
          <ul className="space-y-1.5">
            {insights.map((i) => (
              <li key={i.id} className="flex items-start gap-2 text-[13px] text-ink-100">
                <span className="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-accent" />
                {i.text}
                {i.source === "ai" && <span className="text-2xs text-ink-500">AI</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Component({ label, weight, value }: { label: string; weight: string; value: number | null }) {
  return (
    <div>
      <p className="font-mono text-xl text-ink-50">{fmtScore(value)}</p>
      <p className="text-2xs text-ink-400">{label}</p>
      <p className="text-2xs text-ink-600">{weight}</p>
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between">
      <dt className="text-ink-400">{k}</dt>
      <dd className="font-mono text-ink-100">{v}</dd>
    </div>
  );
}
