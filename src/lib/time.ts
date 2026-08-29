// Time + score formatting helpers used across every view.

export function fmtDuration(totalSecs: number): string {
  const secs = Math.max(0, Math.floor(totalSecs));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

export function fmtClockDuration(totalSecs: number): string {
  const secs = Math.max(0, Math.floor(totalSecs));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${String(h).padStart(2, "0")}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

export function fmtDate(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d) return date;
  return new Date(y, m - 1, d).toLocaleDateString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

export function todayISO(): string {
  return localISO(new Date());
}

export function localISO(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function addDaysISO(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number);
  const dt = new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1);
  dt.setDate(dt.getDate() + days);
  return localISO(dt);
}

/// Minutes-since-midnight → "8:00 AM" style label.
export function fmtMinOfDay(min: number): string {
  const h24 = Math.floor(min / 60) % 24;
  const m = min % 60;
  const d = new Date();
  d.setHours(h24, m, 0, 0);
  return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

export function fmtScore(v: number | null | undefined): string {
  if (v === null || v === undefined) return "—";
  return String(Math.round(v));
}

export function fmtPct(v: number | null | undefined): string {
  if (v === null || v === undefined) return "—";
  return `${Math.round(v)}%`;
}

export function hourLabel(hour: number): string {
  const d = new Date();
  d.setHours(hour, 0, 0, 0);
  return d.toLocaleTimeString([], { hour: "numeric" });
}
