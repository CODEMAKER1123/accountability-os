// Morning Planning Interview (spec §5–6): a conversational, forced-ranking
// flow that ends in a locked daily contract.

import { useEffect, useMemo, useRef, useState } from "react";

import {
  api,
  errorMessage,
  type BreakdownDetail,
  type Commitment,
  type MorningCoach,
  type Priority,
  type Task,
} from "@/lib/ipc";
import {
  applyBreakdownStepsIfCurrent,
  hasCandidateTitle,
  reconcileQuestionOneCandidates,
  type InterviewCandidate,
} from "@/lib/interview";
import { useStore } from "@/lib/store";
import { fmtDuration, todayISO } from "@/lib/time";
import { PriorityTag } from "@/components/shared";

type Candidate = InterviewCandidate;

const DISTRACTION_OPTIONS = [
  "Email",
  "Slack",
  "Random phone calls",
  "Operations problems",
  "Social media",
  "Procrastination",
  "Unexpected meetings",
];

type Step =
  | "intro"
  | "q1"
  | "q2"
  | "q3"
  | "q4"
  | "q5"
  | "q6"
  | "q7"
  | "coach"
  | "contract";

function candidateFromCommitment(commitment: Commitment): Candidate {
  return {
    key: `commitment-${commitment.id}`,
    source: "q1",
    title: commitment.title,
    commitment_id: commitment.id,
    task_id: commitment.task_id,
    selected: true,
    priority: commitment.priority,
    done_definition: commitment.done_definition,
    estimated_minutes: commitment.estimated_minutes,
    steps: commitment.steps.map((step) => step.title),
  };
}

export default function Interview({ mode = "new" }: { mode?: "new" | "edit" }) {
  const { setModal, refreshSnapshot, settings, snapshot } = useStore();
  const editing = mode === "edit";
  const currentPlan = editing ? snapshot?.plan ?? null : null;
  const scheduledWhen = currentPlan?.most_important_when ?? "now";
  const [step, setStep] = useState<Step>(editing ? "q2" : "intro");
  const [mustBeTrue, setMustBeTrue] = useState(() =>
    editing ? (snapshot?.commitments ?? []).map((commitment) => commitment.title).join("\n") : "",
  );
  const [candidates, setCandidates] = useState<Candidate[]>(() =>
    editing ? (snapshot?.commitments ?? []).map(candidateFromCommitment) : [],
  );
  const [customTitle, setCustomTitle] = useState("");
  const [backlog, setBacklog] = useState<Task[]>([]);
  const [limitMessage, setLimitMessage] = useState<string | null>(null);
  const [likelyDistraction, setLikelyDistraction] = useState(
    currentPlan?.likely_distraction ?? "",
  );
  const [countermeasure, setCountermeasure] = useState(currentPlan?.countermeasure ?? "");
  const [whenMostImportant, setWhenMostImportant] = useState(
    scheduledWhen.startsWith("specific:") ? "specific" : scheduledWhen,
  );
  const [specificTime, setSpecificTime] = useState(
    scheduledWhen.startsWith("specific:") ? scheduledWhen.slice("specific:".length) : "09:00",
  );
  const [coach, setCoach] = useState<MorningCoach | null>(null);
  const [coachLoading, setCoachLoading] = useState(false);
  const [breakdownDetail, setBreakdownDetail] = useState<BreakdownDetail>("standard");
  const [breakdownLoading, setBreakdownLoading] = useState<Set<string>>(new Set());
  const [breakdownErrors, setBreakdownErrors] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [locking, setLocking] = useState(false);
  const breakdownRevision = useRef(0);

  useEffect(() => {
    void api.listTasks(null, null).then(setBacklog).catch(() => setBacklog([]));
  }, []);

  const selected = useMemo(() => candidates.filter((c) => c.selected), [candidates]);
  const protectedCommitmentIds = useMemo(
    () =>
      new Set(
        (snapshot?.commitments ?? [])
          .filter(
            (commitment) =>
              commitment.status !== "pending" || commitment.started_at != null,
          )
          .map((commitment) => commitment.id),
      ),
    [snapshot?.commitments],
  );

  const toggleCandidate = async (key: string) => {
    const candidate = candidates.find((item) => item.key === key);
    if (
      editing &&
      candidate?.selected &&
      candidate.commitment_id != null &&
      protectedCommitmentIds.has(candidate.commitment_id)
    ) {
      setLimitMessage("Started or completed commitments stay in today's accountability record.");
      return;
    }
    const next = candidates.map((c) => (c.key === key ? { ...c, selected: !c.selected } : c));
    setCandidates(next);
    const count = next.filter((c) => c.selected).length;
    setLimitMessage(count > 3 ? await api.commitmentLimitCheck(count) : null);
  };

  const addFromQ1 = () => {
    breakdownRevision.current += 1;
    const reconciled = reconcileQuestionOneCandidates(candidates, mustBeTrue);
    if (!editing) {
      setCandidates(reconciled);
      setLimitMessage(null);
      setStep("q2");
      return;
    }
    const retainedIds = new Set(
      reconciled.map((candidate) => candidate.commitment_id).filter((id) => id != null),
    );
    const protectedMissing = candidates.filter(
      (candidate) =>
        candidate.commitment_id != null &&
        protectedCommitmentIds.has(candidate.commitment_id) &&
        !retainedIds.has(candidate.commitment_id),
    );
    if (protectedMissing.length > 0) {
      setLimitMessage(
        "Started or completed commitments were kept so their accountability history stays intact.",
      );
    } else {
      setLimitMessage(null);
    }
    setCandidates([...reconciled, ...protectedMissing]);
    setStep("q2");
  };

  const addCustom = () => {
    if (customTitle.trim().length < 3) return;
    if (hasCandidateTitle(candidates, customTitle)) {
      setCustomTitle("");
      return;
    }
    setCandidates((prev) => [
      ...prev,
      {
        key: `custom-${Date.now()}`,
        source: "custom",
        title: customTitle.trim(),
        commitment_id: null,
        task_id: null,
        selected: prev.filter((c) => c.selected).length < 3,
        priority: "must",
        done_definition: "",
        estimated_minutes: 60,
        steps: [],
      },
    ]);
    setCustomTitle("");
  };

  const addFromBacklog = (t: Task) => {
    if (candidates.some((c) => c.task_id === t.id) || hasCandidateTitle(candidates, t.title)) return;
    setCandidates((prev) => [
      ...prev,
      {
        key: `task-${t.id}`,
        source: "backlog",
        title: t.title,
        commitment_id: null,
        task_id: t.id,
        selected: prev.filter((c) => c.selected).length < 3,
        priority: t.priority,
        done_definition: "",
        estimated_minutes: t.estimated_minutes ?? 60,
        steps: [],
      },
    ]);
  };

  const updateSelected = (key: string, patch: Partial<Candidate>) => {
    setCandidates((prev) => prev.map((c) => (c.key === key ? { ...c, ...patch } : c)));
  };

  const runBreakdown = async (candidate: Candidate) => {
    const requestedRevision = breakdownRevision.current;
    const request = { key: candidate.key, title: candidate.title, steps: candidate.steps };
    setBreakdownErrors((prev) => ({ ...prev, [candidate.key]: "" }));
    setBreakdownLoading((prev) => new Set(prev).add(candidate.key));
    try {
      const result = await api.breakDownGoal(candidate.title, breakdownDetail);
      setCandidates((prev) =>
        breakdownRevision.current === requestedRevision
          ? applyBreakdownStepsIfCurrent(prev, request, result.steps)
          : prev,
      );
    } catch (e) {
      if (breakdownRevision.current === requestedRevision) {
        setBreakdownErrors((prev) => ({ ...prev, [candidate.key]: errorMessage(e) }));
      }
    } finally {
      setBreakdownLoading((prev) => {
        const next = new Set(prev);
        next.delete(candidate.key);
        return next;
      });
    }
  };

  const updateActionStep = (candidate: Candidate, index: number, title: string) => {
    breakdownRevision.current += 1;
    const steps = [...candidate.steps];
    steps[index] = title;
    updateSelected(candidate.key, { steps });
  };

  const removeActionStep = (candidate: Candidate, index: number) => {
    breakdownRevision.current += 1;
    updateSelected(candidate.key, {
      steps: candidate.steps.filter((_, stepIndex) => stepIndex !== index),
    });
  };

  const totalEstimate = selected.reduce((sum, c) => sum + (c.estimated_minutes ?? 0), 0);

  const runCoach = async () => {
    setStep("coach");
    setCoachLoading(true);
    try {
      const result = await api.getMorningCoach(selected.map((c) => c.title));
      setCoach(result);
    } catch {
      setCoach(null);
    } finally {
      setCoachLoading(false);
    }
  };

  const saveDay = async () => {
    setLocking(true);
    setError(null);
    try {
      const input = {
        date: todayISO(),
        commitments: selected.map((c) => ({
          ...(editing ? { id: c.commitment_id } : {}),
          task_id: c.task_id,
          title: c.title,
          done_definition: c.done_definition,
          estimated_minutes: c.estimated_minutes,
          priority: c.priority,
          steps: c.steps.map((step) => step.trim()).filter(Boolean),
        })),
        likely_distraction: likelyDistraction,
        countermeasure,
        most_important_when:
          whenMostImportant === "specific" ? `specific:${specificTime}` : whenMostImportant,
        interview_answers: { must_be_true: mustBeTrue },
      };
      if (editing) {
        await api.reviseDay({
          ...input,
          commitments: selected.map((c) => ({
            id: c.commitment_id,
            task_id: c.task_id,
            title: c.title,
            done_definition: c.done_definition,
            estimated_minutes: c.estimated_minutes,
            priority: c.priority,
            steps: c.steps.map((actionStep) => actionStep.trim()).filter(Boolean),
          })),
        });
      } else {
        await api.lockDay(input);
      }
      await refreshSnapshot();
      setModal(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLocking(false);
    }
  };

  const snooze = async () => {
    const result = await api.snoozeInterview(15);
    if (!result.allowed) {
      setError(result.message);
      return;
    }
    setModal(null);
  };

  const dayOff = async () => {
    await api.markDayOff();
    await refreshSnapshot();
    setModal(null);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div className="flex max-h-full w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-ink-600 bg-ink-900 shadow-2xl">
        <div className="flex items-center justify-between border-b border-ink-700 px-5 py-3">
          <div>
            <p className="text-sm font-semibold text-ink-50">
              {editing ? "Edit Today's Plan" : "Daily Planning Interview"}
            </p>
            {editing && (
              <p className="text-2xs text-ink-500">
                Progress and focus history stay attached while you revise.
              </p>
            )}
          </div>
          <div className="flex items-center gap-3">
            {step !== "intro" && (
              <p className="text-2xs text-ink-500">
                {selected.length > 0 && `${selected.length} commitment${selected.length > 1 ? "s" : ""} selected`}
              </p>
            )}
            {editing && (
              <button className="btn py-1 text-2xs" onClick={() => setModal(null)}>
                Cancel
              </button>
            )}
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {step === "intro" && (
            <div className="space-y-4">
              <p className="text-base text-ink-100">
                Time to plan the day. This takes about three minutes and ends with a locked
                contract of 1–3 outcomes.
              </p>
              {error && <p className="text-xs text-warn">{error}</p>}
              <div className="flex gap-2">
                <button className="btn btn-primary" onClick={() => setStep("q1")}>
                  Start interview
                </button>
                <button className="btn" onClick={() => void snooze()}>
                  Snooze 15 minutes
                </button>
                <button className="btn" onClick={() => void dayOff()}>
                  Today is off / vacation
                </button>
              </div>
              {settings?.strict_mode && (
                <p className="text-2xs text-ink-500">Strict Mode: snoozing is limited to 2 per day.</p>
              )}
            </div>
          )}

          {step === "q1" && (
            <Question
              n={1}
              text="What absolutely must be true by the end of today?"
              hint="Free text. One outcome per line works best — each line becomes a candidate commitment."
            >
              <textarea
                autoFocus
                className="input h-36 resize-none"
                value={mustBeTrue}
                onChange={(e) => {
                  breakdownRevision.current += 1;
                  setMustBeTrue(e.target.value);
                }}
                placeholder={"Finish the commercial sales playbook and send it\nReview Baltimore pipeline\n…"}
              />
              <NavButtons
                onNext={addFromQ1}
                nextDisabled={mustBeTrue.trim().length < 5}
              />
            </Question>
          )}

          {step === "q2" && (
            <Question
              n={2}
              text="Which 1–3 outcomes matter most?"
              hint="Force-rank. More than three means none of them are priorities."
            >
              <div className="space-y-1.5">
                {candidates.map((c) => (
                  <label
                    key={c.key}
                    className={`flex cursor-pointer items-center gap-3 rounded-md border px-3 py-2 ${
                      c.selected ? "border-accent/50 bg-accent/5" : "border-ink-700"
                    }`}
                  >
                    <input
                      type="checkbox"
                      className="h-3.5 w-3.5 accent-[#5b8def]"
                      checked={c.selected}
                      disabled={
                        editing &&
                        c.commitment_id != null &&
                        protectedCommitmentIds.has(c.commitment_id)
                      }
                      onChange={() => void toggleCandidate(c.key)}
                    />
                    <span className="min-w-0 flex-1 truncate text-[13px] text-ink-100">{c.title}</span>
                    {editing &&
                      c.commitment_id != null &&
                      protectedCommitmentIds.has(c.commitment_id) && (
                        <span className="text-2xs text-ink-500">progress kept</span>
                      )}
                    {c.task_id != null && <span className="text-2xs text-ink-500">backlog</span>}
                  </label>
                ))}
              </div>
              {limitMessage && (
                <p className="rounded-md border border-warn/40 bg-warn/10 px-3 py-2 text-xs text-warn">
                  {limitMessage}
                </p>
              )}
              <div className="flex gap-2">
                <input
                  className="input flex-1"
                  placeholder="Add another outcome…"
                  value={customTitle}
                  onChange={(e) => setCustomTitle(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && addCustom()}
                />
                <button className="btn" onClick={addCustom}>
                  Add
                </button>
              </div>
              {backlog.length > 0 && (
                <details className="text-xs text-ink-400">
                  <summary className="cursor-pointer select-none">Pull from backlog…</summary>
                  <div className="mt-2 max-h-36 space-y-1 overflow-y-auto">
                    {backlog
                      .filter((t) => !candidates.some((c) => c.task_id === t.id))
                      .slice(0, 20)
                      .map((t) => (
                        <button
                          key={t.id}
                          className="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-ink-800"
                          onClick={() => addFromBacklog(t)}
                        >
                          <PriorityTag priority={t.priority} />
                          <span className="truncate text-ink-200">{t.title}</span>
                        </button>
                      ))}
                  </div>
                </details>
              )}
              <NavButtons
                onBack={() => {
                  breakdownRevision.current += 1;
                  setStep("q1");
                }}
                onNext={() => setStep("q3")}
                nextDisabled={selected.length === 0 || selected.length > 3}
              />
            </Question>
          )}

          {step === "q3" && (
            <Question
              n={3}
              text="What does DONE mean?"
              hint={'Bad: "Work on commercial sales." Good: "Finish the 10-page playbook and send it to the team."'}
            >
              <div className="flex items-center justify-between rounded-md border border-ink-700 bg-ink-850 px-3 py-2">
                <div>
                  <p className="text-xs font-medium text-ink-200">AI breakdown detail</p>
                  <p className="text-2xs text-ink-500">Generated steps stay editable.</p>
                </div>
                <select
                  aria-label="AI breakdown detail"
                  className="input w-28"
                  value={breakdownDetail}
                  onChange={(event) => {
                    breakdownRevision.current += 1;
                    setBreakdownDetail(event.target.value as BreakdownDetail);
                  }}
                >
                  <option value="simple">Simple</option>
                  <option value="standard">Standard</option>
                  <option value="detailed">Detailed</option>
                </select>
              </div>
              <div className="space-y-3">
                {selected.map((c) => (
                  <div key={c.key} className="rounded-md border border-ink-700 p-3">
                    <p className="mb-1 text-xs font-medium text-ink-200">{c.title}</p>
                    <textarea
                      className="input h-14 resize-none"
                      placeholder="DONE means…"
                      value={c.done_definition}
                      onChange={(e) => updateSelected(c.key, { done_definition: e.target.value })}
                    />
                    <div className="mt-3 flex flex-wrap items-center gap-2">
                      <button
                        className="btn py-1"
                        onClick={() => void runBreakdown(c)}
                        disabled={breakdownLoading.has(c.key) || !settings?.ai_coaching_enabled}
                        title={
                          settings?.ai_coaching_enabled
                            ? "Generate an editable checklist"
                            : "Enable AI coaching in Settings first"
                        }
                      >
                        {breakdownLoading.has(c.key) ? "Breaking down…" : "Break into steps with AI"}
                      </button>
                      <button
                        className="btn py-1"
                        onClick={() => {
                          if (c.steps.length >= 12) return;
                          breakdownRevision.current += 1;
                          updateSelected(c.key, { steps: [...c.steps, ""] });
                        }}
                        disabled={c.steps.length >= 12}
                      >
                        Add step
                      </button>
                      {!settings?.ai_coaching_enabled && (
                        <span className="text-2xs text-ink-500">AI coaching is off in Settings.</span>
                      )}
                    </div>
                    {breakdownErrors[c.key] && (
                      <p className="mt-2 text-2xs text-distracted">{breakdownErrors[c.key]}</p>
                    )}
                    {c.steps.length > 0 && (
                      <ol className="mt-2 space-y-1.5">
                        {c.steps.map((actionStep, index) => (
                          <li key={`${c.key}-step-${index}`} className="flex items-center gap-2">
                            <span className="w-4 text-right font-mono text-2xs text-ink-500">
                              {index + 1}.
                            </span>
                            <input
                              aria-label={`Step ${index + 1} for ${c.title}`}
                              className="input flex-1 py-1.5"
                              value={actionStep}
                              onChange={(event) => updateActionStep(c, index, event.target.value)}
                            />
                            <button
                              aria-label={`Remove step ${index + 1}`}
                              className="btn px-2 py-1 text-ink-400"
                              onClick={() => removeActionStep(c, index)}
                            >
                              Remove
                            </button>
                          </li>
                        ))}
                      </ol>
                    )}
                  </div>
                ))}
              </div>
              <NavButtons
                onBack={() => {
                  breakdownRevision.current += 1;
                  setStep("q2");
                }}
                onNext={() => {
                  breakdownRevision.current += 1;
                  setStep("q4");
                }}
                nextDisabled={selected.some((c) => c.done_definition.trim().length < 10)}
                nextHint={
                  selected.some((c) => c.done_definition.trim().length < 10)
                    ? "Each commitment needs a real completion definition (a sentence, not a phrase)."
                    : undefined
                }
              />
            </Question>
          )}

          {step === "q4" && (
            <Question n={4} text="Estimate required focused time." hint="Honest numbers beat hopeful ones — estimates feed your planning-accuracy score.">
              <div className="space-y-2">
                {selected.map((c) => (
                  <div key={c.key} className="flex items-center gap-3">
                    <span className="min-w-0 flex-1 truncate text-[13px] text-ink-100">{c.title}</span>
                    <input
                      type="number"
                      min={5}
                      step={5}
                      className="input w-24 text-right"
                      value={c.estimated_minutes ?? ""}
                      onChange={(e) =>
                        updateSelected(c.key, {
                          estimated_minutes: e.target.value === "" ? null : Number(e.target.value),
                        })
                      }
                    />
                    <span className="w-16 text-xs text-ink-500">minutes</span>
                    <select
                      className="input w-24"
                      value={c.priority}
                      onChange={(e) => updateSelected(c.key, { priority: e.target.value as Priority })}
                    >
                      <option value="must">must</option>
                      <option value="should">should</option>
                      <option value="could">could</option>
                    </select>
                  </div>
                ))}
              </div>
              <p className="text-xs text-ink-400">
                Total estimated focus: <span className="text-ink-100">{fmtDuration(totalEstimate * 60)}</span>
              </p>
              <NavButtons onBack={() => setStep("q3")} onNext={() => setStep("q5")} />
            </Question>
          )}

          {step === "q5" && (
            <Question n={5} text="What is most likely to derail you today?">
              <div className="flex flex-wrap gap-1.5">
                {DISTRACTION_OPTIONS.map((d) => (
                  <button
                    key={d}
                    className={`rounded-full border px-3 py-1 text-xs ${
                      likelyDistraction === d
                        ? "border-accent bg-accent/10 text-ink-50"
                        : "border-ink-600 text-ink-300 hover:border-ink-500"
                    }`}
                    onClick={() => setLikelyDistraction(d)}
                  >
                    {d}
                  </button>
                ))}
              </div>
              <input
                className="input"
                placeholder="…or describe it"
                value={likelyDistraction}
                onChange={(e) => setLikelyDistraction(e.target.value)}
              />
              <NavButtons
                onBack={() => setStep("q4")}
                onNext={() => setStep("q6")}
                nextDisabled={likelyDistraction.trim().length < 3}
              />
            </Question>
          )}

          {step === "q6" && (
            <Question
              n={6}
              text="What will you do when that happens?"
              hint="This countermeasure is shown back to you when drift is detected."
            >
              <textarea
                autoFocus
                className="input h-20 resize-none"
                placeholder="Capture the issue in the inbox and return to the current priority."
                value={countermeasure}
                onChange={(e) => setCountermeasure(e.target.value)}
              />
              <NavButtons
                onBack={() => setStep("q5")}
                onNext={() => setStep("q7")}
                nextDisabled={countermeasure.trim().length < 5}
              />
            </Question>
          )}

          {step === "q7" && (
            <Question n={7} text="When should you work on the most important thing?">
              <div className="space-y-1.5">
                {[
                  ["now", "Now"],
                  ["before_lunch", "Before lunch"],
                  ["specific", "Specific time"],
                  ["flexible", "Flexible"],
                ].map(([id, label]) => (
                  <label
                    key={id}
                    className={`flex cursor-pointer items-center gap-3 rounded-md border px-3 py-2 text-[13px] ${
                      whenMostImportant === id ? "border-accent/50 bg-accent/5 text-ink-50" : "border-ink-700 text-ink-200"
                    }`}
                  >
                    <input
                      type="radio"
                      name="when"
                      className="accent-[#5b8def]"
                      checked={whenMostImportant === id}
                      onChange={() => setWhenMostImportant(id!)}
                    />
                    {label}
                    {id === "specific" && whenMostImportant === "specific" && (
                      <input
                        type="time"
                        className="input ml-2 w-28"
                        value={specificTime}
                        onChange={(e) => setSpecificTime(e.target.value)}
                      />
                    )}
                  </label>
                ))}
              </div>
              <NavButtons onBack={() => setStep("q6")} onNext={() => void runCoach()} />
            </Question>
          )}

          {step === "coach" && (
            <Question n={8} text="Coach's read on this plan">
              {coachLoading ? (
                <p className="text-xs text-ink-400">Checking your history…</p>
              ) : coach && coach.text.trim() ? (
                <div className="space-y-2">
                  <div className="whitespace-pre-wrap rounded-md border border-ink-700 bg-ink-850 p-3 text-[13px] leading-relaxed text-ink-100">
                    {coach.text}
                  </div>
                  <p className="text-2xs text-ink-500">
                    {coach.source === "ai" ? "AI coach, grounded in your history" : "From your history"}
                    {coach.avg_completed_per_day != null &&
                      ` · avg ${coach.avg_completed_per_day.toFixed(1)} completed/day`}
                  </p>
                </div>
              ) : (
                <p className="text-xs text-ink-500">
                  Not enough history for pushback yet. It gets sharper after a few planned days.
                </p>
              )}
              <NavButtons onBack={() => setStep("q7")} onNext={() => setStep("contract")} nextLabel="Review contract" />
            </Question>
          )}

          {step === "contract" && (
            <div className="space-y-4">
              <div>
                <p className="section-title">
                  {editing ? "Revised contract" : "Today's contract"}
                </p>
                {editing && (
                  <p className="mt-1 text-2xs text-ink-500">
                    Saving keeps started and completed commitments, focus sessions, and checked
                    steps in place.
                  </p>
                )}
              </div>
              <ol className="space-y-2">
                {selected.map((c, i) => (
                  <li key={c.key} className="flex items-start gap-3 rounded-md border border-ink-700 bg-ink-850 px-3 py-2">
                    <span className="font-mono text-sm text-ink-500">{i + 1}.</span>
                    <div className="min-w-0">
                      <p className="text-[13px] font-medium text-ink-50">{c.title}</p>
                      <p className="text-2xs text-ink-400">
                        DONE = {c.done_definition} · ~{c.estimated_minutes ?? 0}m focused
                      </p>
                      {c.steps.filter((actionStep) => actionStep.trim()).length > 0 && (
                        <ol className="mt-1 list-decimal space-y-0.5 pl-4 text-2xs text-ink-500">
                          {c.steps
                            .filter((actionStep) => actionStep.trim())
                            .map((actionStep, stepIndex) => (
                              <li key={`${c.key}-contract-step-${stepIndex}`}>{actionStep}</li>
                            ))}
                        </ol>
                      )}
                    </div>
                    <span className="ml-auto shrink-0">
                      <PriorityTag priority={c.priority} />
                    </span>
                  </li>
                ))}
              </ol>
              <div className="grid grid-cols-2 gap-3 text-xs">
                <div className="rounded-md border border-ink-700 p-3">
                  <p className="section-title mb-1">Estimated focused work</p>
                  <p className="text-ink-100">{fmtDuration(totalEstimate * 60)}</p>
                </div>
                <div className="rounded-md border border-ink-700 p-3">
                  <p className="section-title mb-1">Likely distraction</p>
                  <p className="text-ink-100">{likelyDistraction}</p>
                  <p className="mt-1 text-ink-400">Countermeasure: {countermeasure}</p>
                </div>
              </div>
              {error && <p className="text-xs text-distracted">{error}</p>}
              <div className="flex items-center justify-between">
                <button className="btn" onClick={() => setStep("coach")}>
                  Back
                </button>
                <button className="btn btn-primary px-6 py-2 text-sm" onClick={() => void saveDay()} disabled={locking}>
                  {locking
                    ? editing
                      ? "Saving…"
                      : "Locking…"
                    : editing
                      ? "SAVE REVISED PLAN"
                      : "LOCK MY DAY"}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function Question({
  n,
  text,
  hint,
  children,
}: {
  n: number;
  text: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-3">
      <div>
        <p className="text-2xs font-semibold uppercase tracking-widest text-ink-500">
          Question {n}
        </p>
        <h2 className="mt-1 text-base font-semibold text-ink-50">{text}</h2>
        {hint && <p className="mt-1 text-xs text-ink-400">{hint}</p>}
      </div>
      {children}
    </div>
  );
}

function NavButtons({
  onBack,
  onNext,
  nextDisabled,
  nextLabel = "Next",
  nextHint,
}: {
  onBack?: () => void;
  onNext: () => void;
  nextDisabled?: boolean;
  nextLabel?: string;
  nextHint?: string;
}) {
  return (
    <div className="space-y-1 pt-2">
      {nextHint && <p className="text-2xs text-ink-500">{nextHint}</p>}
      <div className="flex justify-between">
        {onBack ? (
          <button className="btn" onClick={onBack}>
            Back
          </button>
        ) : (
          <span />
        )}
        <button className="btn btn-primary" onClick={onNext} disabled={nextDisabled}>
          {nextLabel}
        </button>
      </div>
    </div>
  );
}
