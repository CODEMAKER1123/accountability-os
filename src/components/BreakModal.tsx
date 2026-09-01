// Planned breaks (spec §17): never counted as distraction.

import { useState } from "react";

import Dialog from "@/components/Dialog";
import { api, errorMessage } from "@/lib/ipc";
import { useStore } from "@/lib/store";

export default function BreakModal() {
  const { setModal, refreshSnapshot } = useStore();
  const [custom, setCustom] = useState("");
  const [error, setError] = useState<string | null>(null);

  const start = async (minutes: number) => {
    try {
      await api.startBreak(minutes);
      await refreshSnapshot();
      setModal(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Dialog labelledBy="take-break-title" onClose={() => setModal(null)} panelClassName="w-full max-w-sm rounded-xl border border-ink-600 bg-ink-900 p-5 shadow-2xl">
        <p id="take-break-title" className="text-sm font-semibold text-ink-50">Take a break</p>
        <p className="mt-1 text-xs text-ink-400">
          Planned breaks pause distraction tracking. You'll be notified when it ends.
        </p>
        <div className="mt-4 grid grid-cols-4 gap-2">
          {[5, 10, 15, 30].map((m) => (
            <button key={m} className="btn" onClick={() => void start(m)}>
              {m} min
            </button>
          ))}
        </div>
        <div className="mt-2 flex gap-2">
          <input
            type="number"
            min={1}
            max={240}
            className="input flex-1"
            placeholder="Custom minutes"
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
          />
          <button
            className="btn"
            disabled={!custom || Number(custom) < 1}
            onClick={() => void start(Number(custom))}
          >
            Start
          </button>
        </div>
        {error && <p className="mt-2 text-xs text-distracted">{error}</p>}
        <div className="mt-4 flex justify-end">
          <button className="btn" onClick={() => setModal(null)}>
            Cancel
          </button>
        </div>
    </Dialog>
  );
}
