// Priority-change flow (spec §15): intentional, logged, never punished as
// distraction — but it always requires a reason.

import { useState } from "react";

import { api, errorMessage } from "@/lib/ipc";
import { useStore } from "@/lib/store";

const DISPOSITIONS: [string, string][] = [
  ["still_today", "Still completing today"],
  ["later", "Move to later"],
  ["defer", "Defer to another day"],
  ["cancel", "Cancel it"],
];

export default function SwitchModal({
  fromCommitmentId,
  toCommitmentId = null,
  toTaskId = null,
  toTaskTitle,
}: {
  fromCommitmentId: number | null;
  toCommitmentId?: number | null;
  toTaskId?: number | null;
  toTaskTitle?: string;
}) {
  const { setModal, refreshSnapshot, snapshot } = useStore();
  const [reason, setReason] = useState("");
  const [toId, setToId] = useState<number | null>(toCommitmentId);
  const [targetTaskId, setTargetTaskId] = useState<number | null>(toTaskId);
  const [disposition, setDisposition] = useState("still_today");
  const [error, setError] = useState<string | null>(null);

  // The commitment being switched away from may not be in today's list —
  // an overnight focus session leaves yesterday's commitment active. Fall
  // back to the snapshot's active commitment so the disposition is still
  // collected instead of silently defaulting.
  const fromToday = snapshot?.commitments.find((c) => c.id === fromCommitmentId);
  const fromTitle =
    fromToday?.title ??
    (snapshot?.active_commitment && snapshot.active_commitment.id === fromCommitmentId
      ? snapshot.active_commitment.title
      : null);
  const options = (snapshot?.commitments ?? []).filter(
    (c) =>
      c.id !== fromCommitmentId &&
      c.status !== "completed" &&
      c.status !== "cancelled" &&
      c.status !== "deferred",
  );

  const submit = async () => {
    try {
      await api.switchCommitment({
        to_commitment_id: targetTaskId == null ? toId : null,
        to_task_id: targetTaskId,
        from_commitment_id: fromCommitmentId,
        reason,
        original_disposition: disposition,
      });
      await refreshSnapshot();
      setModal(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div className="w-full max-w-md rounded-xl border border-ink-600 bg-ink-900 p-5 shadow-2xl">
        <p className="text-sm font-semibold text-ink-50">Switch priority</p>
        {fromTitle && <p className="mt-1 text-xs text-ink-400">Currently: {fromTitle}</p>}

        <label className="label mt-4">What changed?</label>
        <input
          autoFocus
          className="input"
          placeholder="Required — e.g. client escalation, deadline moved"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />

        <label className="label mt-3">What are you working on instead?</label>
        <div className="space-y-1">
          {toTaskId != null && (
            <label
              className={`flex cursor-pointer items-center gap-2 rounded-md border px-3 py-1.5 text-[13px] ${
                targetTaskId === toTaskId
                  ? "border-accent/50 bg-accent/5 text-ink-50"
                  : "border-ink-700 text-ink-200"
              }`}
            >
              <input
                type="radio"
                name="to"
                className="accent-[#5b8def]"
                checked={targetTaskId === toTaskId}
                onChange={() => {
                  setTargetTaskId(toTaskId);
                  setToId(null);
                }}
              />
              {toTaskTitle ?? "Selected task"}
            </label>
          )}
          {options.map((c) => (
            <label
              key={c.id}
              className={`flex cursor-pointer items-center gap-2 rounded-md border px-3 py-1.5 text-[13px] ${
                toId === c.id ? "border-accent/50 bg-accent/5 text-ink-50" : "border-ink-700 text-ink-200"
              }`}
            >
              <input
                type="radio"
                name="to"
                className="accent-[#5b8def]"
                checked={targetTaskId == null && toId === c.id}
                onChange={() => {
                  setTargetTaskId(null);
                  setToId(c.id);
                }}
              />
              {c.title}
            </label>
          ))}
          <label
            className={`flex cursor-pointer items-center gap-2 rounded-md border px-3 py-1.5 text-[13px] ${
              targetTaskId == null && toId === null
                ? "border-accent/50 bg-accent/5 text-ink-50"
                : "border-ink-700 text-ink-200"
            }`}
          >
            <input
              type="radio"
              name="to"
              className="accent-[#5b8def]"
              checked={targetTaskId == null && toId === null}
              onChange={() => {
                setTargetTaskId(null);
                setToId(null);
              }}
            />
            Something off-plan (pause the contract)
          </label>
        </div>

        {fromCommitmentId != null && fromTitle && (
          <>
            <label className="label mt-3">What happens to "{fromTitle}"?</label>
            <div className="grid grid-cols-2 gap-1">
              {DISPOSITIONS.map(([id, label]) => (
                <label
                  key={id}
                  className={`flex cursor-pointer items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs ${
                    disposition === id ? "border-accent/50 bg-accent/5 text-ink-50" : "border-ink-700 text-ink-300"
                  }`}
                >
                  <input
                    type="radio"
                    name="disp"
                    className="accent-[#5b8def]"
                    checked={disposition === id}
                    onChange={() => setDisposition(id!)}
                  />
                  {label}
                </label>
              ))}
            </div>
          </>
        )}

        {error && <p className="mt-2 text-xs text-distracted">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setModal(null)}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={() => void submit()} disabled={reason.trim().length < 3}>
            Switch
          </button>
        </div>
      </div>
    </div>
  );
}
