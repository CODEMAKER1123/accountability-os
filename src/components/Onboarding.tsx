// First-run onboarding (spec §40): explicit privacy disclosure, work hours,
// cadence, strict mode, AI, extension, live monitoring test, first task.

import { useEffect, useState } from "react";

import Dialog from "@/components/Dialog";
import { api, errorMessage, type MonitoringStatus, type Settings } from "@/lib/ipc";
import { useStore } from "@/lib/store";

const STEPS = [
  "welcome",
  "privacy",
  "schedule",
  "strictness",
  "ai",
  "extension",
  "test",
  "first-task",
] as const;

type Step = (typeof STEPS)[number];

export default function Onboarding() {
  const { setModal, loadSettings, refreshSnapshot } = useStore();
  const [step, setStep] = useState<Step>("welcome");
  const [form, setForm] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api.getSettings().then(setForm);
  }, []);

  if (!form) return null;
  const patch = (p: Partial<Settings>) => setForm({ ...form, ...p });
  const idx = STEPS.indexOf(step);
  const go = (delta: number) => setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, idx + delta))]!);

  const finish = async () => {
    try {
      await api.updateSettings({ ...form, onboarding_completed: true });
      await loadSettings();
      await refreshSnapshot();
      setModal(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Dialog labelledBy="onboarding-title" panelClassName="flex max-h-full w-full max-w-xl flex-col overflow-hidden rounded-xl border border-ink-600 bg-ink-900 shadow-2xl">
        <div className="border-b border-ink-700 px-5 py-3">
          <div className="flex items-center justify-between">
            <p id="onboarding-title" className="text-sm font-semibold text-ink-50">Set up Accountability OS</p>
            <p className="text-2xs text-ink-500">
              {idx + 1} / {STEPS.length}
            </p>
          </div>
          <div className="mt-2 h-0.5 w-full rounded bg-ink-700">
            <div className="h-full rounded bg-accent" style={{ width: `${((idx + 1) / STEPS.length) * 100}%` }} />
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {step === "welcome" && (
            <div className="space-y-3">
              <h2 className="text-lg font-semibold text-ink-50">
                Did you actually spend the day on what you said was important?
              </h2>
              <p className="text-[13px] leading-relaxed text-ink-300">
                Accountability OS is not a to-do app. Each morning you commit to 1–3 outcomes. It
                then watches which applications you actually use, tells you when you drift, checks
                in periodically, and scores the day against your own contract.
              </p>
              <p className="text-[13px] text-ink-300">
                Busy does not equal productive. This measures the difference.
              </p>
            </div>
          )}

          {step === "privacy" && (
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-ink-50">Desktop monitoring & privacy</h2>
              <p className="text-[13px] text-ink-300">
                To detect drift, the app reads which window is in the foreground. That requires your
                explicit consent.
              </p>
              <div className="grid grid-cols-2 gap-3 text-xs">
                <div className="rounded-md border border-focus/30 bg-focus/5 p-3">
                  <p className="mb-1 font-semibold text-focus">Collected (locally)</p>
                  <ul className="space-y-0.5 text-ink-300">
                    <li>Active application + window title</li>
                    <li>Start/end timestamps, durations</li>
                    <li>Idle time, lock state</li>
                    <li>Active tab domain/title (if extension installed)</li>
                    <li>Your commitments and answers</li>
                  </ul>
                </div>
                <div className="rounded-md border border-distracted/30 bg-distracted/5 p-3">
                  <p className="mb-1 font-semibold text-distracted">Never collected</p>
                  <ul className="space-y-0.5 text-ink-300">
                    <li>Keystrokes, passwords, clipboard</li>
                    <li>Message or email bodies</li>
                    <li>Microphone, camera, screenshots</li>
                    <li>Raw browser history</li>
                  </ul>
                </div>
              </div>
              <p className="text-xs text-ink-400">
                Everything stays in a local SQLite database on this machine. If you enable AI, only
                the active commitment plus the current window context is sent for classification.
                You can exclude or mark apps as private, and delete history at any time.
              </p>
              <label className="flex cursor-pointer items-center gap-2 rounded-md border border-ink-600 bg-ink-850 p-3 text-[13px] text-ink-100">
                <input
                  type="checkbox"
                  className="h-4 w-4 accent-[#4ea87c]"
                  checked={form.monitoring_consent}
                  onChange={(e) => patch({ monitoring_consent: e.target.checked })}
                />
                I consent to local desktop activity monitoring.
              </label>
            </div>
          )}

          {step === "schedule" && (
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-ink-50">Your work rhythm</h2>
              <div className="grid grid-cols-2 gap-3">
                <TimeInput label="Workday starts" value={form.work_start_min} onChange={(v) => patch({ work_start_min: v })} />
                <TimeInput label="Workday ends" value={form.work_end_min} onChange={(v) => patch({ work_end_min: v })} />
                <TimeInput label="Morning interview" value={form.interview_time_min} onChange={(v) => patch({ interview_time_min: v })} />
                <TimeInput label="End-of-day review" value={form.review_time_min} onChange={(v) => patch({ review_time_min: v })} />
              </div>
              <div>
                <label className="label">Accountability check-ins</label>
                <div className="flex gap-2">
                  {[60, 90, 120].map((m) => (
                    <button
                      key={m}
                      className={`btn ${form.checkin_cadence_min === m ? "btn-primary" : ""}`}
                      onClick={() => patch({ checkin_cadence_min: m })}
                    >
                      Every {m} min
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}

          {step === "strictness" && (
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-ink-50">How strict should it be?</h2>
              <button
                className={`w-full rounded-md border p-3 text-left ${!form.strict_mode ? "border-accent/50 bg-accent/5" : "border-ink-700"}`}
                onClick={() => patch({ strict_mode: false })}
              >
                <p className="text-[13px] font-medium text-ink-50">Standard</p>
                <p className="text-xs text-ink-400">
                  Prompts can be dismissed; snoozing is unlimited. Good starting point.
                </p>
              </button>
              <button
                className={`w-full rounded-md border p-3 text-left ${form.strict_mode ? "border-accent/50 bg-accent/5" : "border-ink-700"}`}
                onClick={() => patch({ strict_mode: true })}
              >
                <p className="text-[13px] font-medium text-ink-50">Strict Mode</p>
                <p className="text-xs text-ink-400">
                  Morning interview can't be endlessly snoozed, accountability prompts stay visible
                  until answered, and switching commitments requires an explanation. It never locks
                  your computer.
                </p>
              </button>
            </div>
          )}

          {step === "ai" && (
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-ink-50">AI (optional)</h2>
              <p className="text-xs text-ink-400">
                Everything works without AI: rules and your corrections handle classification, and
                scoring is fully deterministic. With an OpenAI-compatible API key the app also
                classifies ambiguous activity, coaches your morning plan, and analyzes your day. You
                can configure this later in Settings → AI.
              </p>
              <label className="label">Base URL</label>
              <input className="input" value={form.ai_base_url} onChange={(e) => patch({ ai_base_url: e.target.value })} />
              <OnboardingKeyField
                onStored={() => patch({ ai_classification_enabled: true, ai_coaching_enabled: true })}
              />
              {form.ai_classification_enabled && (
                <p className="text-xs text-focus">AI classification + coaching enabled.</p>
              )}
            </div>
          )}

          {step === "extension" && (
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-ink-50">Browser extension (optional)</h2>
              <p className="text-xs text-ink-400">
                Without it, browser activity is classified by window title only. The extension adds
                the active tab's domain and title — active tab only, focused window only, sent only
                to this machine.
              </p>
              <ol className="list-decimal space-y-1 pl-5 text-xs text-ink-300">
                <li>Open chrome://extensions (or edge://extensions) and enable Developer mode.</li>
                <li>
                  Click "Load unpacked" and select the <code className="text-ink-200">extension/</code>{" "}
                  folder from the install directory.
                </li>
                <li>Open the extension's options and paste the pairing token from Settings.</li>
              </ol>
              <p className="text-2xs text-ink-500">You can do this any time later.</p>
            </div>
          )}

          {step === "test" && (
            <MonitorTest
              consent={form.monitoring_consent}
              demo={form.demo_mode}
              onDemoToggle={(v) => {
                patch({ demo_mode: v });
                void api.setDemoMode(v); // takes effect immediately for the test
              }}
            />
          )}

          {step === "first-task" && <FirstTask />}
        </div>

        <div className="flex items-center justify-between border-t border-ink-700 px-5 py-3">
          <button className="btn" onClick={() => go(-1)} disabled={idx === 0}>
            Back
          </button>
          {error && <p className="max-w-xs truncate text-xs text-distracted">{error}</p>}
          {step === "first-task" ? (
            <button className="btn btn-primary" onClick={() => void finish()}>
              Finish setup
            </button>
          ) : (
            <button
              className="btn btn-primary"
              onClick={() => go(1)}
              disabled={step === "privacy" && !form.monitoring_consent && !form.demo_mode}
            >
              {step === "privacy" && !form.monitoring_consent ? "Consent required to continue" : "Continue"}
            </button>
          )}
        </div>
    </Dialog>
  );
}

function TimeInput({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
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
    </div>
  );
}

function OnboardingKeyField({ onStored }: { onStored: () => void }) {
  const [key, setKey] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  return (
    <div>
      <label className="label">API key</label>
      <div className="flex gap-2">
        <input type="password" className="input flex-1" placeholder="sk-… (leave empty to skip)" value={key} onChange={(e) => setKey(e.target.value)} />
        <button
          className="btn"
          disabled={!key.trim()}
          onClick={async () => {
            try {
              await api.setAiKey(key);
              setMsg("Stored in OS credential storage.");
              onStored();
              setKey("");
            } catch (e) {
              setMsg(errorMessage(e));
            }
          }}
        >
          Save
        </button>
      </div>
      {msg && <p className="mt-1 text-xs text-ink-300">{msg}</p>}
    </div>
  );
}

/// Live proof that monitoring works (spec §40 step 11).
function MonitorTest({
  consent,
  demo,
  onDemoToggle,
}: {
  consent: boolean;
  demo: boolean;
  onDemoToggle: (v: boolean) => void;
}) {
  const [status, setStatus] = useState<MonitoringStatus | null>(null);
  useEffect(() => {
    // Consent may not be persisted yet mid-onboarding; grant it for the test.
    if (consent) void api.grantMonitoringConsent();
    const t = setInterval(() => void api.getMonitoringStatus().then(setStatus), 1500);
    return () => clearInterval(t);
  }, [consent]);
  return (
    <div className="space-y-3">
      <h2 className="text-base font-semibold text-ink-50">Monitoring test</h2>
      <p className="text-xs text-ink-400">
        Switch to another window for a few seconds, then come back. The detected activity should
        change.
      </p>
      <div className="rounded-md border border-ink-700 bg-ink-850 p-4">
        {status?.current ? (
          <>
            <p className="text-sm font-medium text-ink-50">
              {status.current.is_idle ? "Idle" : status.current.app_name}
            </p>
            <p className="truncate text-xs text-ink-400">{status.current.window_title || "—"}</p>
            <p className="mt-1 text-2xs text-focus">Monitoring works.</p>
          </>
        ) : (
          <p className="text-xs text-ink-400">
            {status?.message ?? "Waiting for the first sample…"}
          </p>
        )}
      </div>
      {status?.state === "permission_required" && (
        <label className="flex cursor-pointer items-center gap-2 text-xs text-ink-300">
          <input type="checkbox" className="accent-[#5b8def]" checked={demo} onChange={(e) => onDemoToggle(e.target.checked)} />
          Native monitoring isn't available on this OS — enable Demo Mode to simulate activity.
        </label>
      )}
    </div>
  );
}

function FirstTask() {
  const [title, setTitle] = useState("");
  const [created, setCreated] = useState<string | null>(null);
  return (
    <div className="space-y-3">
      <h2 className="text-base font-semibold text-ink-50">Capture your first task</h2>
      <p className="text-xs text-ink-400">
        Anything on your mind. From now on, Ctrl+Shift+Space captures from anywhere.
      </p>
      <div className="flex gap-2">
        <input
          autoFocus
          className="input flex-1"
          placeholder="e.g. Draft the commercial sales playbook"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={async (e) => {
            if (e.key === "Enter" && title.trim()) {
              await api.createTask({ title: title.trim() });
              setCreated(title.trim());
              setTitle("");
            }
          }}
        />
        <button
          className="btn"
          disabled={!title.trim()}
          onClick={async () => {
            await api.createTask({ title: title.trim() });
            setCreated(title.trim());
            setTitle("");
          }}
        >
          Add
        </button>
      </div>
      {created && <p className="text-xs text-focus">Added "{created}" to your inbox.</p>}
    </div>
  );
}
