// Quick Capture (spec §4): Ctrl+Shift+Space from anywhere.

import { useEffect, useRef, useState } from "react";

import { api, type Priority } from "@/lib/ipc";

export default function CaptureWindow() {
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("should");
  const [saved, setSaved] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void api.closeWindow("capture");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const save = async (andClose: boolean) => {
    if (!title.trim()) return;
    await api.createTask({ title: title.trim(), priority, status: "inbox" });
    setSaved(title.trim());
    setTitle("");
    if (andClose) {
      await api.closeWindow("capture");
    } else {
      inputRef.current?.focus();
      setTimeout(() => setSaved(null), 1600);
    }
  };

  return (
    <div className="flex h-screen flex-col gap-2 border border-ink-600 bg-ink-950 p-4" data-tauri-drag-region>
      <div className="flex items-center justify-between" data-tauri-drag-region>
        <p className="text-2xs font-semibold uppercase tracking-widest text-ink-400">Quick capture</p>
        <button className="text-ink-600 hover:text-ink-300" onClick={() => void api.closeWindow("capture")}>
          ✕
        </button>
      </div>
      <div className="flex gap-2">
        <input
          ref={inputRef}
          className="input flex-1 text-sm"
          placeholder="What's on your mind? Enter saves · Shift+Enter saves & keeps capturing"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save(!e.shiftKey);
          }}
        />
        <select className="input w-28" value={priority} onChange={(e) => setPriority(e.target.value as Priority)}>
          <option value="must">Must</option>
          <option value="should">Should</option>
          <option value="could">Could</option>
        </select>
      </div>
      <p className="text-2xs text-ink-500">
        {saved ? `Captured "${saved}" → inbox.` : "Goes to your task inbox. Esc closes."}
      </p>
    </div>
  );
}
