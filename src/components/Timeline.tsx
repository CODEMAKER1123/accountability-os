// Visual chronological timeline (spec §44): rows per app/domain, colored
// segments per classification, clickable.

import { useMemo } from "react";

import { CLASS_BG } from "@/components/shared";
import type { ActivitySession, TimelineData } from "@/lib/ipc";
import { fmtDuration, fmtTime, hourLabel } from "@/lib/time";

const MAX_ROWS = 8;

export default function Timeline({
  data,
  onSelect,
}: {
  data: TimelineData;
  onSelect: (s: ActivitySession) => void;
}) {
  const { rows, rangeStart, rangeEnd } = useMemo(() => {
    const sessions = data.sessions;
    if (sessions.length === 0) {
      return { rows: [], rangeStart: 0, rangeEnd: 1 };
    }
    // Range: from first to last activity, padded to hour boundaries.
    const first = Math.min(...sessions.map((s) => s.started_at));
    const last = Math.max(...sessions.map((s) => s.ended_at));
    const rangeStart = Math.floor(first / 3600) * 3600;
    const rangeEnd = Math.ceil(last / 3600) * 3600;

    // Group rows by app (or domain), largest total first; overflow → Other.
    const totals = new Map<string, number>();
    const labelOf = (s: ActivitySession) =>
      s.is_idle ? "Idle" : (s.browser_domain ?? s.application_name);
    for (const s of sessions) {
      totals.set(labelOf(s), (totals.get(labelOf(s)) ?? 0) + s.duration_seconds);
    }
    const ranked = [...totals.entries()].sort((a, b) => b[1] - a[1]);
    const keep = new Set(ranked.slice(0, MAX_ROWS).map(([k]) => k));
    const rowMap = new Map<string, { label: string; total: number; sessions: ActivitySession[] }>();
    for (const s of sessions) {
      const label = keep.has(labelOf(s)) ? labelOf(s) : "Other";
      const row = rowMap.get(label) ?? { label, total: 0, sessions: [] };
      row.total += s.duration_seconds;
      row.sessions.push(s);
      rowMap.set(label, row);
    }
    const rows = [...rowMap.values()].sort((a, b) => {
      if (a.label === "Idle") return 1;
      if (b.label === "Idle") return -1;
      return b.total - a.total;
    });
    return { rows, rangeStart, rangeEnd };
  }, [data]);

  if (rows.length === 0) return null;
  const span = Math.max(1, rangeEnd - rangeStart);
  const pct = (ts: number) => ((ts - rangeStart) / span) * 100;

  const hourMarks: number[] = [];
  for (let t = rangeStart; t <= rangeEnd; t += 3600) hourMarks.push(t);
  const markStep = Math.max(1, Math.ceil(hourMarks.length / 10));

  return (
    <div className="card">
      <div className="mb-2 flex justify-between">
        <p className="section-title">Timeline</p>
        <p className="text-2xs text-ink-500">
          {fmtTime(rangeStart)} — {fmtTime(rangeEnd)}
        </p>
      </div>
      {/* Hour axis */}
      <div className="relative ml-32 h-4">
        {hourMarks.map(
          (t, i) =>
            i % markStep === 0 && (
              <span
                key={t}
                className="absolute -translate-x-1/2 text-2xs text-ink-500"
                style={{ left: `${pct(t)}%` }}
              >
                {hourLabel(new Date(t * 1000).getHours())}
              </span>
            ),
        )}
      </div>
      <div className="space-y-1.5">
        {rows.map((row) => (
          <div key={row.label} className="flex items-center gap-2">
            <span className="w-[7.5rem] shrink-0 truncate text-right text-2xs text-ink-400">
              {row.label}
            </span>
            <div className="relative h-4 flex-1 rounded bg-ink-850">
              {hourMarks.map((t) => (
                <span
                  key={t}
                  className="absolute top-0 h-full w-px bg-ink-800"
                  style={{ left: `${pct(t)}%` }}
                />
              ))}
              {row.sessions.map((s) => (
                <button
                  key={s.id}
                  className={`absolute top-0 h-full min-w-[2px] rounded-sm opacity-80 transition-opacity hover:opacity-100 ${CLASS_BG[s.classification]}`}
                  style={{
                    left: `${pct(s.started_at)}%`,
                    width: `${Math.max(0.15, pct(s.ended_at) - pct(s.started_at))}%`,
                  }}
                  title={`${fmtTime(s.started_at)}–${fmtTime(s.ended_at)} · ${
                    s.window_title || row.label
                  } · ${s.classification} · ${fmtDuration(s.duration_seconds)}${
                    s.classification_reason ? `\n${s.classification_reason}` : ""
                  }`}
                  onClick={() => onSelect(s)}
                />
              ))}
            </div>
            <span className="w-12 shrink-0 text-right font-mono text-2xs text-ink-500">
              {fmtDuration(row.total)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
