// Small shared UI atoms. Color logic per spec §39: subdued, not a scoreboard.

import type { Classification, MonitoringState } from "@/lib/ipc";

export const CLASS_COLORS: Record<Classification, string> = {
  focused: "text-focus",
  supporting: "text-supporting",
  neutral: "text-neutralcat",
  distracted: "text-distracted",
  idle: "text-idlecat",
  unknown: "text-ink-400",
};

export const CLASS_BG: Record<Classification, string> = {
  focused: "bg-focus",
  supporting: "bg-supporting",
  neutral: "bg-neutralcat",
  distracted: "bg-distracted",
  idle: "bg-idlecat",
  unknown: "bg-ink-600",
};

export const CLASS_LABEL: Record<Classification, string> = {
  focused: "Focused",
  supporting: "Supporting",
  neutral: "Neutral",
  distracted: "Distracted",
  idle: "Idle",
  unknown: "Unclassified",
};

export function ClassBadge({ classification }: { classification: Classification }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 text-2xs font-semibold uppercase tracking-wider ${CLASS_COLORS[classification]}`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${CLASS_BG[classification]}`} />
      {CLASS_LABEL[classification]}
    </span>
  );
}

export function MonitoringBadge({
  state,
  message,
}: {
  state: MonitoringState;
  message?: string | null;
}) {
  const config: Record<MonitoringState, { label: string; cls: string }> = {
    active: { label: "Monitoring active", cls: "text-focus" },
    demo: { label: "Demo simulation", cls: "text-accent" },
    paused: { label: "Monitoring paused", cls: "text-warn" },
    permission_required: { label: "Permission required", cls: "text-distracted" },
  };
  const c = config[state];
  return (
    <span className={`inline-flex items-center gap-1.5 text-2xs font-medium ${c.cls}`} title={message ?? undefined}>
      <span className="relative flex h-1.5 w-1.5">
        <span className={`h-1.5 w-1.5 rounded-full ${state === "active" || state === "demo" ? "bg-current" : "bg-current opacity-60"}`} />
      </span>
      {c.label}
    </span>
  );
}

export function PriorityTag({ priority }: { priority: string }) {
  const cls =
    priority === "must"
      ? "border-distracted/40 text-distracted"
      : priority === "should"
        ? "border-warn/40 text-warn"
        : "border-ink-600 text-ink-400";
  return (
    <span className={`rounded border px-1 py-px text-2xs font-semibold uppercase ${cls}`}>
      {priority}
    </span>
  );
}

export function ProgressBar({
  value,
  max,
  className = "",
}: {
  value: number;
  max: number;
  className?: string;
}) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  return (
    <div className={`h-1 w-full overflow-hidden rounded-full bg-ink-700 ${className}`}>
      <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
    </div>
  );
}

/// Score ring: shows the number, colored by band but subdued.
export function ScoreRing({ value, size = 64, label }: { value: number | null; size?: number; label: string }) {
  const v = value ?? 0;
  const r = (size - 8) / 2;
  const c = 2 * Math.PI * r;
  const color = value === null ? "#4a5561" : v >= 75 ? "#4ea87c" : v >= 50 ? "#d9a052" : "#c96f5e";
  return (
    <div className="flex flex-col items-center gap-1">
      <svg width={size} height={size} className="-rotate-90">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="#232a32" strokeWidth={5} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={color}
          strokeWidth={5}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c - (c * Math.min(100, Math.max(0, v))) / 100}
        />
        <text
          x="50%"
          y="50%"
          className="rotate-90"
          textAnchor="middle"
          dominantBaseline="central"
          transform={`rotate(90 ${size / 2} ${size / 2})`}
          fill="#dde2e8"
          fontSize={size / 4}
          fontWeight={600}
        >
          {value === null ? "—" : Math.round(v)}
        </text>
      </svg>
      <span className="text-2xs uppercase tracking-wider text-ink-400">{label}</span>
    </div>
  );
}

export function EmptyState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 py-16 text-center">
      <p className="text-sm font-medium text-ink-300">{title}</p>
      {hint && <p className="max-w-sm text-xs text-ink-500">{hint}</p>}
    </div>
  );
}

export function ErrorBanner({
  message,
  onDismiss,
  onRetry,
}: {
  message: string;
  onDismiss?: () => void;
  onRetry?: () => void;
}) {
  return (
    <div
      className="flex items-start justify-between gap-3 rounded-md border border-distracted/40 bg-distracted/10 px-3 py-2 text-xs text-ink-100"
      role="alert"
    >
      <p className="min-w-0 leading-relaxed">{message}</p>
      <div className="flex shrink-0 gap-2">
        {onRetry && (
          <button className="font-medium text-ink-100 hover:text-white" onClick={onRetry}>
            Retry
          </button>
        )}
        {onDismiss && (
          <button
            className="text-ink-400 hover:text-ink-100"
            aria-label="Dismiss error"
            onClick={onDismiss}
          >
            ✕
          </button>
        )}
      </div>
    </div>
  );
}
