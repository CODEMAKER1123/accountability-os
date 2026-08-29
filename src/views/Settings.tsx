// Settings (spec §51) — work rhythm, accountability, monitoring & privacy,
// classification rules, AI, extension bridge, data lifecycle.

import { useEffect, useState } from "react";

import {
  api,
  errorMessage,
  type AppRule,
  type Classification,
  type DomainRule,
  type Settings as SettingsModel,
} from "@/lib/ipc";
import { useStore } from "@/lib/store";
import { fmtMinOfDay } from "@/lib/time";

export default function Settings() {
  const { settings, loadSettings, setSettings, refreshSnapshot, snapshot } = useStore();
  const [form, setForm] = useState<SettingsModel | null>(settings);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!settings) void loadSettings().then(setForm);
    else setForm(settings);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!form) return <div className="p-6 text-ink-400">Loading…</div>;

  const patch = (p: Partial<SettingsModel>) => setForm({ ...form, ...p });

  const save = async () => {
    setError(null);
    setStatus(null);
    let previousAutostart: boolean | null = null;
    let settingsSaved = false;
    try {
      const auto = await import("@tauri-apps/plugin-autostart");
      previousAutostart = await auto.isEnabled();
      if (previousAutostart !== form.launch_at_startup) {
        if (form.launch_at_startup) await auto.enable();
        else await auto.disable();
      }
      const saved = await api.updateSettings(form);
      settingsSaved = true;
      setSettings(saved);
      setForm(saved);
      await api.setWidgetVisible(saved.widget_enabled);
      await refreshSnapshot();
      setStatus("Saved.");
      setTimeout(() => setStatus(null), 2500);
    } catch (e) {
      if (!settingsSaved && previousAutostart != null) {
        try {
          const auto = await import("@tauri-apps/plugin-autostart");
          if (previousAutostart) await auto.enable();
          else await auto.disable();
        } catch {
          // Keep the original error; a later save can reconcile startup state.
        }
      }
      setError(errorMessage(e));
    }
  };

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-6 pb-24">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-ink-50">Settings</h1>
        <div className="flex items-center gap-3">
          {status && <span className="text-xs text-focus">{status}</span>}
          {error && <span className="max-w-md truncate text-xs text-distracted">{error}</span>}
          <button className="btn btn-primary" onClick={() => void save()}>
            Save changes
          </button>
        </div>
      </div>

      {/* Work rhythm */}
      <section className="card space-y-3">
        <p className="section-title">Work rhythm</p>
        <div className="grid grid-cols-3 gap-3">
          <TimeField label="Workday start" value={form.work_start_min} onChange={(v) => patch({ work_start_min: v })} />
          <TimeField label="Workday end" value={form.work_end_min} onChange={(v) => patch({ work_end_min: v })} />
          <TimeField label="Morning interview" value={form.interview_time_min} onChange={(v) => patch({ interview_time_min: v })} />
          <TimeField label="End-of-day review" value={form.review_time_min} onChange={(v) => patch({ review_time_min: v })} />
          <div>
            <label className="label">Check-in cadence</label>
            <select
              className="input"
              value={form.checkin_cadence_min}
              onChange={(e) => patch({ checkin_cadence_min: Number(e.target.value) })}
            >
              {[60, 90, 120].map((m) => (
                <option key={m} value={m}>
                  Every {m} minutes
                </option>
              ))}
            </select>
          </div>
        </div>
      </section>

      {/* Accountability */}
      <section className="card space-y-3">
        <p className="section-title">Accountability</p>
        <div className="grid grid-cols-3 gap-3">
          <div>
            <label className="label">Distraction warning</label>
            <select
              className="input"
              value={form.distraction_warn_secs}
              onChange={(e) => patch({ distraction_warn_secs: Number(e.target.value) })}
            >
              {[60, 120, 180, 300].map((s) => (
                <option key={s} value={s}>
                  After {s / 60} min
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="label">Intervention</label>
            <select
              className="input"
              value={form.distraction_intervene_secs}
              onChange={(e) => patch({ distraction_intervene_secs: Number(e.target.value) })}
            >
              {[300, 420, 600, 900].map((s) => (
                <option key={s} value={s}>
                  After {s / 60} min
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="label">Idle threshold</label>
            <select
              className="input"
              value={form.idle_threshold_secs}
              onChange={(e) => patch({ idle_threshold_secs: Number(e.target.value) })}
            >
              {[60, 120, 180, 300, 600].map((s) => (
                <option key={s} value={s}>
                  {s / 60} min without input
                </option>
              ))}
            </select>
          </div>
        </div>
        <Toggle
          label="Strict Mode"
          hint="Interview can't be endlessly snoozed, prompts stay until answered, switching requires explanation. Persistent, never hostile — it never locks your computer."
          checked={form.strict_mode}
          onChange={(v) => patch({ strict_mode: v })}
        />
      </section>

      {/* Monitoring & privacy */}
      <section className="card space-y-3">
        <div className="flex items-center justify-between">
          <p className="section-title">Monitoring & privacy</p>
          {snapshot && (
            <button
              className="btn py-1 text-xs"
              onClick={async () => {
                if (snapshot.monitoring_state === "paused") await api.resumeMonitoring();
                else await api.pauseMonitoring();
                await refreshSnapshot();
              }}
            >
              {snapshot.monitoring_state === "paused" ? "Resume monitoring" : "Pause monitoring"}
            </button>
          )}
        </div>
        <p className="text-xs text-ink-400">
          Only the active app, window title, timestamps, idle and lock state are recorded — locally.
          No keystrokes, no clipboard, no message bodies, no screenshots. See PRIVACY.md.
        </p>
        <Toggle
          label="Desktop activity monitoring"
          hint="Revoking consent stops new desktop activity capture. Previously recorded history remains until you delete it below."
          checked={form.monitoring_consent}
          onChange={(v) => patch({ monitoring_consent: v })}
        />
        <Toggle
          label="Browser monitoring (extension)"
          hint="Accept active-tab domain + title from the local browser extension."
          checked={form.browser_monitoring_enabled}
          onChange={(v) => patch({ browser_monitoring_enabled: v })}
        />
        <Toggle
          label="Demo Mode"
          hint="Replace the OS probe with a scripted simulation (focus, a long distraction, idle). For trying the product and development."
          checked={form.demo_mode}
          onChange={(v) => patch({ demo_mode: v })}
        />
        <ListField
          label="Excluded applications (never recorded at all)"
          placeholder="e.g. keepass.exe"
          values={form.excluded_apps}
          onChange={(v) => patch({ excluded_apps: v })}
        />
        <ListField
          label="Excluded domains (never recorded at all)"
          placeholder="e.g. mybank.com"
          values={form.excluded_domains}
          onChange={(v) => patch({ excluded_domains: v })}
        />
        <ListField
          label='Private applications (recorded as "Private Application", no titles)'
          placeholder="e.g. 1password.exe"
          values={form.private_apps}
          onChange={(v) => patch({ private_apps: v })}
        />
        <div>
          <label className="label">Keep activity history for</label>
          <select
            className="input w-56"
            value={form.activity_retention_days}
            onChange={(e) => patch({ activity_retention_days: Number(e.target.value) })}
          >
            {[30, 90, 180, 365, 730].map((d) => (
              <option key={d} value={d}>
                {d} days
              </option>
            ))}
          </select>
        </div>
        <DataControls />
      </section>

      <RulesEditor />

      {/* AI */}
      <section className="card space-y-3">
        <p className="section-title">AI (OpenAI-compatible)</p>
        <Toggle
          label="AI activity classification"
          hint="Ambiguous activity (not settled by rules or your corrections) is classified by the AI. Only the active commitment + current window context is sent."
          checked={form.ai_classification_enabled}
          onChange={(v) => patch({ ai_classification_enabled: v })}
        />
        <Toggle
          label="AI coaching"
          hint="Morning coach, end-of-day analysis, insight narration."
          checked={form.ai_coaching_enabled}
          onChange={(v) => patch({ ai_coaching_enabled: v })}
        />
        <div className="grid grid-cols-3 gap-3">
          <div className="col-span-3">
            <label className="label">Base URL</label>
            <input className="input" value={form.ai_base_url} onChange={(e) => patch({ ai_base_url: e.target.value })} />
          </div>
          <div>
            <label className="label">Classification model</label>
            <input className="input" value={form.ai_classify_model} onChange={(e) => patch({ ai_classify_model: e.target.value })} />
          </div>
          <div>
            <label className="label">Coaching model</label>
            <input className="input" value={form.ai_coach_model} onChange={(e) => patch({ ai_coach_model: e.target.value })} />
          </div>
        </div>
        <ApiKeyField />
      </section>

      {/* App behavior */}
      <section className="card space-y-3">
        <p className="section-title">App behavior</p>
        <Toggle
          label="Launch at startup"
          checked={form.launch_at_startup}
          onChange={(v) => patch({ launch_at_startup: v })}
        />
        <Toggle label="Start minimized to tray" checked={form.start_minimized} onChange={(v) => patch({ start_minimized: v })} />
        <Toggle
          label="Always-on-top focus widget"
          hint="Small corner widget with the active commitment and timer."
          checked={form.widget_enabled}
          onChange={(v) => patch({ widget_enabled: v })}
        />
        <Toggle
          label="Widget stays on top"
          checked={form.widget_always_on_top}
          onChange={(v) => patch({ widget_always_on_top: v })}
        />
      </section>

      <ExtensionSection />
    </div>
  );
}

function TimeField({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
  const h = String(Math.floor(value / 60)).padStart(2, "0");
  const m = String(value % 60).padStart(2, "0");
  return (
    <div>
      <label className="label">{label}</label>
      <input
        type="time"
        className="input"
        value={`${h}:${m}`}
        onChange={(e) => {
          const [hh, mm] = e.target.value.split(":").map(Number);
          onChange((hh ?? 0) * 60 + (mm ?? 0));
        }}
      />
      <p className="mt-0.5 text-2xs text-ink-500">{fmtMinOfDay(value)}</p>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3">
      <input
        type="checkbox"
        className="mt-0.5 h-3.5 w-3.5 accent-[#5b8def]"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>
        <span className="block text-[13px] text-ink-100">{label}</span>
        {hint && <span className="block text-2xs text-ink-500">{hint}</span>}
      </span>
    </label>
  );
}

function ListField({
  label,
  placeholder,
  values,
  onChange,
}: {
  label: string;
  placeholder: string;
  values: string[];
  onChange: (v: string[]) => void;
}) {
  const [draft, setDraft] = useState("");
  return (
    <div>
      <label className="label">{label}</label>
      <div className="flex flex-wrap gap-1.5">
        {values.map((v) => (
          <span key={v} className="inline-flex items-center gap-1 rounded border border-ink-600 bg-ink-800 px-2 py-0.5 text-xs text-ink-200">
            {v}
            <button className="text-ink-500 hover:text-distracted" onClick={() => onChange(values.filter((x) => x !== v))}>
              ✕
            </button>
          </span>
        ))}
        <input
          className="input w-56"
          placeholder={placeholder}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) {
              onChange([...new Set([...values, draft.trim().toLowerCase()])]);
              setDraft("");
            }
          }}
        />
      </div>
    </div>
  );
}

function ApiKeyField() {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [draft, setDraft] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void api.hasAiKey().then(setHasKey).catch(() => setHasKey(false));
  }, []);
  return (
    <div className="space-y-2">
      <label className="label">
        API key {hasKey ? "· stored in OS credential storage" : "· not set"}
      </label>
      <div className="flex gap-2">
        <input
          type="password"
          className="input flex-1"
          placeholder={hasKey ? "•••••••• (enter a new key to replace, empty to remove)" : "sk-…"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button
          className="btn"
          onClick={async () => {
            try {
              const stored = await api.setAiKey(draft);
              setHasKey(stored);
              setDraft("");
              setMsg(stored ? "Key stored securely." : "Key removed.");
            } catch (e) {
              setMsg(errorMessage(e));
            }
          }}
        >
          Save key
        </button>
        <button
          className="btn"
          disabled={busy || !hasKey}
          onClick={async () => {
            setBusy(true);
            setMsg("Testing…");
            try {
              setMsg(await api.testAiConnection());
            } catch (e) {
              setMsg(errorMessage(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          Test connection
        </button>
      </div>
      {msg && <p className="text-xs text-ink-300">{msg}</p>}
    </div>
  );
}

function RulesEditor() {
  const [domainRules, setDomainRules] = useState<DomainRule[]>([]);
  const [appRules, setAppRules] = useState<AppRule[]>([]);
  const [draftDomain, setDraftDomain] = useState("");
  const [draftClass, setDraftClass] = useState<Classification>("distracted");
  const [draftFocusOnly, setDraftFocusOnly] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = async () => {
    const r = await api.listRules();
    setDomainRules(r.domain_rules);
    setAppRules(r.app_rules);
  };
  useEffect(() => {
    void reload();
  }, []);

  return (
    <section className="card space-y-3">
      <p className="section-title">Classification rules</p>
      <p className="text-xs text-ink-400">
        Deterministic layer 1: these settle a classification before corrections or AI are consulted.
        Defaults mark common social/news sites as distraction during focus sessions.
      </p>
      <div className="flex gap-2">
        <input
          className="input flex-1"
          placeholder="domain, e.g. news.ycombinator.com"
          value={draftDomain}
          onChange={(e) => setDraftDomain(e.target.value)}
        />
        <select className="input w-36" value={draftClass} onChange={(e) => setDraftClass(e.target.value as Classification)}>
          {["focused", "supporting", "neutral", "distracted"].map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-1.5 text-xs text-ink-300">
          <input type="checkbox" checked={draftFocusOnly} onChange={(e) => setDraftFocusOnly(e.target.checked)} className="accent-[#5b8def]" />
          focus only
        </label>
        <button
          className="btn"
          onClick={async () => {
            try {
              await api.addDomainRule(draftDomain, draftClass, draftFocusOnly);
              setDraftDomain("");
              setError(null);
              await reload();
            } catch (e) {
              setError(errorMessage(e));
            }
          }}
        >
          Add rule
        </button>
      </div>
      {error && <p className="text-xs text-distracted">{error}</p>}
      <div className="max-h-48 space-y-1 overflow-y-auto">
        {domainRules.map((r) => (
          <div key={`d${r.id}`} className="flex items-center gap-2 rounded border border-ink-700 px-2 py-1 text-xs">
            <span className="flex-1 truncate text-ink-100">{r.domain}</span>
            <span className="text-ink-400">{r.classification}</span>
            {r.only_in_focus && <span className="text-2xs text-ink-500">focus only</span>}
            {r.is_default && <span className="text-2xs text-ink-600">default</span>}
            <button className="text-ink-500 hover:text-distracted" onClick={async () => { await api.deleteRule("domain", r.id); await reload(); }}>
              ✕
            </button>
          </div>
        ))}
        {appRules.map((r) => (
          <div key={`a${r.id}`} className="flex items-center gap-2 rounded border border-ink-700 px-2 py-1 text-xs">
            <span className="flex-1 truncate text-ink-100">{r.process_name}</span>
            <span className="text-ink-400">{r.classification}</span>
            {r.only_in_focus && <span className="text-2xs text-ink-500">focus only</span>}
            <button className="text-ink-500 hover:text-distracted" onClick={async () => { await api.deleteRule("app", r.id); await reload(); }}>
              ✕
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function DataControls() {
  const [msg, setMsg] = useState<string | null>(null);
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [confirmScope, setConfirmScope] = useState<"today" | "range" | "all" | null>(null);
  const del = async (scope: "today" | "range" | "all") => {
    try {
      const n = await api.deleteActivity(
        scope,
        scope === "range" ? fromDate : undefined,
        scope === "range" ? toDate : undefined,
      );
      setMsg(`Deleted ${n} activity sessions and their derived monitoring data.`);
      setConfirmScope(null);
    } catch (e) {
      setMsg(errorMessage(e));
    }
  };
  return (
    <div className="space-y-2 border-t border-ink-700 pt-3">
      <p className="text-2xs uppercase tracking-wider text-ink-500">Your data</p>
      <div className="flex flex-wrap gap-2">
        <button className="btn" onClick={() => setConfirmScope("today")}>
          Delete today's activity
        </button>
        <button
          className="btn"
          disabled={!fromDate || !toDate || fromDate > toDate}
          onClick={() => setConfirmScope("range")}
        >
          Delete date range
        </button>
        <button className="btn" onClick={() => setConfirmScope("all")}>
          Delete all monitoring history
        </button>
        <button
          className="btn"
          onClick={async () => {
            try {
              const dialog = await import("@tauri-apps/plugin-dialog");
              const path = await dialog.save({
                defaultPath: "accountability-os-export.json",
                filters: [{ name: "JSON", extensions: ["json"] }],
              });
              if (path) {
                await api.exportData(path);
                setMsg(`Exported to ${path}`);
              }
            } catch (e) {
              setMsg(errorMessage(e));
            }
          }}
        >
          Export my data
        </button>
      </div>
      <div className="flex flex-wrap items-end gap-2">
        <label className="text-xs text-ink-400">
          From
          <input
            className="input mt-1 w-40"
            type="date"
            value={fromDate}
            onChange={(event) => setFromDate(event.target.value)}
          />
        </label>
        <label className="text-xs text-ink-400">
          Through
          <input
            className="input mt-1 w-40"
            type="date"
            value={toDate}
            onChange={(event) => setToDate(event.target.value)}
          />
        </label>
        {fromDate && toDate && fromDate > toDate && (
          <span className="pb-2 text-xs text-distracted">Start must be before end.</span>
        )}
      </div>
      {confirmScope && (
        <div className="flex items-center gap-2 rounded-md border border-distracted/40 bg-distracted/10 px-3 py-2 text-xs">
          <span className="text-ink-100">
            Permanently delete{" "}
            {confirmScope === "today"
              ? "today's activity"
              : confirmScope === "range"
                ? `monitoring history from ${fromDate} through ${toDate}`
                : "ALL monitoring history"}
            ?
          </span>
          <button className="btn btn-danger py-1" onClick={() => void del(confirmScope)}>
            Delete
          </button>
          <button className="btn py-1" onClick={() => setConfirmScope(null)}>
            Cancel
          </button>
        </div>
      )}
      {msg && <p className="text-xs text-ink-300">{msg}</p>}
    </div>
  );
}

function ExtensionSection() {
  const [info, setInfo] = useState<{ port: number; token: string; connected: boolean } | null>(null);
  const [revealed, setRevealed] = useState(false);
  useEffect(() => {
    void api.getExtensionInfo().then(setInfo);
    const t = setInterval(() => void api.getExtensionInfo().then(setInfo), 5000);
    return () => clearInterval(t);
  }, []);
  if (!info) return null;
  return (
    <section className="card space-y-2">
      <div className="flex items-center justify-between">
        <p className="section-title">Browser extension</p>
        <span className={`text-xs ${info.connected ? "text-focus" : "text-ink-500"}`}>
          {info.connected ? "Connected" : "Not connected"}
        </span>
      </div>
      <p className="text-xs text-ink-400">
        Install the extension from the <code className="text-ink-300">extension/</code> folder
        (chrome://extensions → Load unpacked), then paste the pairing token into its options page.
        It reports only the active tab of a focused browser window, only to this machine.
      </p>
      <dl className="space-y-1 text-xs">
        <div className="flex gap-3">
          <dt className="text-ink-500">Local port</dt>
          <dd className="font-mono text-ink-200">{info.port}</dd>
        </div>
        <div className="flex items-center gap-3">
          <dt className="text-ink-500">Pairing token</dt>
          <dd className="font-mono text-ink-200">{revealed ? info.token : "••••••••••••"}</dd>
          <button className="btn-ghost btn px-2 py-0.5 text-2xs" onClick={() => setRevealed(!revealed)}>
            {revealed ? "Hide" : "Reveal"}
          </button>
          <button
            className="btn-ghost btn px-2 py-0.5 text-2xs"
            onClick={() => void navigator.clipboard.writeText(info.token)}
          >
            Copy
          </button>
        </div>
      </dl>
    </section>
  );
}
