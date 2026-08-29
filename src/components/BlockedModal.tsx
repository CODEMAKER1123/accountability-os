// Blocked flow (spec §16): name the blocker, find the smallest unblocking
// action, and capture it as a task.

import { useState } from "react";

import { api, errorMessage } from "@/lib/ipc";
import { useStore } from "@/lib/store";

const BLOCKERS: [string, string][] = [
  ["waiting_for_someone", "Waiting for someone"],
  ["need_information", "Need information"],
  ["technical_issue", "Technical issue"],
  ["need_decision", "Need a decision"],
  ["dont_know_next", "Don't know what to do next"],
  ["other", "Other"],
];

export default function BlockedModal({ commitmentId }: { commitmentId: number }) {
  const { setModal, refreshSnapshot, snapshot } = useStore();
  const [kind, setKind] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [nextAction, setNextAction] = useState("");
  const [error, setError] = useState<string | null>(null);

  const commitment = snapshot?.commitments.find((c) => c.id === commitmentId);

  const submit = async () => {
    if (!kind) return;
    try {
      const created = await api.blockCommitment({
        commitment_id: commitmentId,
        blocker_kind: kind,
        note: note || null,
        next_action: nextAction || null,
      });
      await refreshSnapshot();
      setModal(null);
      void created;
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div className="w-full max-w-md rounded-xl border border-ink-600 bg-ink-900 p-5 shadow-2xl">
        <p className="text-sm font-semibold text-ink-50">I'm blocked</p>
        {commitment && <p className="mt-1 text-xs text-ink-400">{commitment.title}</p>}

        <label className="label mt-4">What is blocking you?</label>
        <div className="grid grid-cols-2 gap-1">
          {BLOCKERS.map(([id, label]) => (
            <label
              key={id}
              className={`flex cursor-pointer items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs ${
                kind === id ? "border-accent/50 bg-accent/5 text-ink-50" : "border-ink-700 text-ink-300"
              }`}
            >
              <input type="radio" name="blocker" className="accent-[#5b8def]" checked={kind === id} onChange={() => setKind(id!)} />
              {label}
            </label>
          ))}
        </div>

        <label className="label mt-3">Details (optional)</label>
        <input className="input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="Waiting on legal review from Dana…" />

        <label className="label mt-3">
          What is the smallest next action that would unblock this?
        </label>
        <input
          className="input"
          value={nextAction}
          onChange={(e) => setNextAction(e.target.value)}
          placeholder='e.g. "Send Dana a 2-line status ping" — becomes a must-do task'
        />

        {error && <p className="mt-2 text-xs text-distracted">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setModal(null)}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={() => void submit()} disabled={!kind}>
            Mark blocked
          </button>
        </div>
      </div>
    </div>
  );
}
