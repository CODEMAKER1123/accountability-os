import { useEffect, useRef, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";

import {
  checkForAppUpdate,
  downloadAndInstallAppUpdate,
  EMPTY_UPDATE_PROGRESS,
  invalidateAppUpdate,
  type UpdateDownloadProgress,
} from "@/lib/appUpdater";

type InstallPhase = "ready" | "checking" | "downloading" | "installing" | "restarting" | "failed";

const STARTUP_CHECK_DELAY_MS = 2_000;
const BACKGROUND_CHECK_INTERVAL_MS = 4 * 60 * 60_000;

export default function AppUpdater() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [phase, setPhase] = useState<InstallPhase>("ready");
  const [progress, setProgress] = useState<UpdateDownloadProgress>(EMPTY_UPDATE_PROGRESS);
  const [failure, setFailure] = useState<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    const checkNow = async () => {
      try {
        const available = await checkForAppUpdate();
        if (mounted.current && available) {
          setUpdate(available);
          setFailure(null);
          setPhase((current) => (current === "failed" ? "ready" : current));
        }
      } catch (error) {
        // Startup checks stay quiet while offline. The next background check
        // retries without interrupting the user's work.
        console.warn("update check failed", error);
      }
    };
    const initial = window.setTimeout(() => void checkNow(), STARTUP_CHECK_DELAY_MS);
    const interval = window.setInterval(() => void checkNow(), BACKGROUND_CHECK_INTERVAL_MS);
    window.addEventListener("focus", checkNow);
    return () => {
      mounted.current = false;
      window.clearTimeout(initial);
      window.clearInterval(interval);
      window.removeEventListener("focus", checkNow);
    };
  }, []);

  if (!update) return null;

  const install = async () => {
    if (phase !== "ready" && phase !== "failed") return;
    let candidate = update;
    setFailure(null);
    setProgress(EMPTY_UPDATE_PROGRESS);
    try {
      if (phase === "failed") {
        setPhase("checking");
        const refreshed = await checkForAppUpdate(true);
        if (!mounted.current) return;
        if (!refreshed) {
          setUpdate(null);
          setPhase("ready");
          return;
        }
        candidate = refreshed;
        setUpdate(refreshed);
      }

      setPhase("downloading");
      await downloadAndInstallAppUpdate(candidate, (nextProgress) => {
        if (!mounted.current) return;
        setProgress(nextProgress);
        if (nextProgress.finished) setPhase("installing");
      });
      if (!mounted.current) return;
      setPhase("restarting");
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (error) {
      invalidateAppUpdate(candidate);
      if (!mounted.current) return;
      setFailure(error instanceof Error ? error.message : String(error));
      setPhase("failed");
    }
  };

  const busy =
    phase === "checking" ||
    phase === "downloading" ||
    phase === "installing" ||
    phase === "restarting";
  const label =
    phase === "checking"
      ? "Checking for update"
      : phase === "downloading"
      ? progress.percent == null
        ? "Downloading update"
        : `Downloading ${progress.percent}%`
      : phase === "installing"
        ? "Installing update"
        : phase === "restarting"
          ? "Restarting app"
          : phase === "failed"
            ? "Update failed"
            : "Update ready";

  return (
    <div
      className="flex items-center gap-2 rounded-lg border border-accent/35 bg-ink-850 p-2 shadow-lg shadow-black/20"
      aria-live="polite"
    >
      <div className="min-w-0 flex-1">
        <p
          className={`truncate text-xs font-medium ${phase === "failed" ? "text-distracted" : "text-ink-100"}`}
        >
          {label}
        </p>
        <p className="truncate text-2xs text-ink-500">
          {phase === "failed" ? "Click to retry" : `Version ${update.version}`}
        </p>
      </div>
      <button
        type="button"
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent text-white shadow-md shadow-accent/25 transition hover:bg-[#6b9cff] focus:outline-none focus:ring-2 focus:ring-accent/60 disabled:cursor-wait"
        aria-label={phase === "failed" ? "Retry app update" : `Update to version ${update.version}`}
        title={failure ? `Update failed: ${failure}` : `Install Accountability OS ${update.version}`}
        onClick={() => void install()}
        disabled={busy}
      >
        {busy ? (
          <svg className="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <circle className="opacity-25" cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" />
            <path className="opacity-90" d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
          </svg>
        ) : (
          <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path d="M12 4v10m0 0 4-4m-4 4-4-4M5 19h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        )}
      </button>
    </div>
  );
}
