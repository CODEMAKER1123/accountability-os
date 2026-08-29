import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

const CHECK_TIMEOUT_MS = 15_000;
const DOWNLOAD_TIMEOUT_MS = 5 * 60_000;
const MIN_CHECK_INTERVAL_MS = 5 * 60_000;

let readyUpdate: Update | null = null;
let pendingCheck: Promise<Update | null> | null = null;
let lastCheckedAt = 0;

export interface UpdateDownloadProgress {
  downloaded: number;
  total: number | null;
  percent: number | null;
  finished: boolean;
}

export const EMPTY_UPDATE_PROGRESS: UpdateDownloadProgress = {
  downloaded: 0,
  total: null,
  percent: null,
  finished: false,
};

export function reduceDownloadProgress(
  current: UpdateDownloadProgress,
  event: DownloadEvent,
): UpdateDownloadProgress {
  switch (event.event) {
    case "Started": {
      const total = event.data.contentLength ?? null;
      return { downloaded: 0, total, percent: total ? 0 : null, finished: false };
    }
    case "Progress": {
      const downloaded = current.downloaded + event.data.chunkLength;
      const percent = current.total
        ? Math.min(100, Math.round((downloaded / current.total) * 100))
        : null;
      return { ...current, downloaded, percent };
    }
    case "Finished":
      return { ...current, percent: 100, finished: true };
  }
}

export async function checkForAppUpdate(force = false): Promise<Update | null> {
  if (pendingCheck) return pendingCheck;
  if (!force && Date.now() - lastCheckedAt < MIN_CHECK_INTERVAL_MS) return readyUpdate;

  // Count failed/offline attempts too, otherwise repeatedly focusing the app
  // could hammer the update endpoint while the network is unavailable.
  lastCheckedAt = Date.now();
  pendingCheck = check({ timeout: CHECK_TIMEOUT_MS })
    .then((update) => {
      readyUpdate = update;
      return update;
    })
    .finally(() => {
      pendingCheck = null;
    });
  return pendingCheck;
}

export function invalidateAppUpdate(update: Update): void {
  if (readyUpdate === update) {
    readyUpdate = null;
    lastCheckedAt = 0;
  }
}

export async function downloadAndInstallAppUpdate(
  update: Update,
  onProgress: (progress: UpdateDownloadProgress) => void,
): Promise<void> {
  let progress = EMPTY_UPDATE_PROGRESS;
  await update.downloadAndInstall(
    (event) => {
      progress = reduceDownloadProgress(progress, event);
      onProgress(progress);
    },
    { timeout: DOWNLOAD_TIMEOUT_MS },
  );
}
