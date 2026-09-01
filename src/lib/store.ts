// App-level state for the main window: navigation, snapshot, settings, modals.

import { create } from "zustand";

import { api, errorMessage, type Settings, type TodaySnapshot } from "./ipc";

export type View = "today" | "tasks" | "plan" | "activity" | "scorecard" | "settings";

export type Modal =
  | { kind: "interview"; mode?: "new" | "edit" }
  | { kind: "review" }
  | {
      kind: "switch";
      fromCommitmentId: number | null;
      toCommitmentId?: number | null;
      toTaskId?: number | null;
      toTaskTitle?: string;
    }
  | { kind: "blocked"; commitmentId: number }
  | { kind: "break" }
  | { kind: "onboarding" }
  | null;

export interface ToastMessage {
  id: number;
  message: string;
  action?: { label: string; run: () => void | Promise<void> };
}

interface Store {
  view: View;
  setView: (v: View) => void;
  modal: Modal;
  setModal: (m: Modal) => void;
  snapshot: TodaySnapshot | null;
  snapshotLoading: boolean;
  snapshotError: string | null;
  refreshSnapshot: () => Promise<void>;
  settings: Settings | null;
  loadSettings: () => Promise<Settings>;
  setSettings: (s: Settings) => void;
  toast: ToastMessage | null;
  showToast: (message: string, action?: ToastMessage["action"]) => void;
  dismissToast: () => void;
}

let nextToastId = 1;

export const useStore = create<Store>((set) => ({
  view: "today",
  setView: (view) => set({ view }),
  modal: null,
  setModal: (modal) => set({ modal }),
  snapshot: null,
  snapshotLoading: false,
  snapshotError: null,
  refreshSnapshot: async () => {
    set({ snapshotLoading: true });
    try {
      const snapshot = await api.getTodaySnapshot();
      set({ snapshot, snapshotError: null });
    } catch (e) {
      console.error("snapshot refresh failed", e);
      set({ snapshotError: errorMessage(e) });
    } finally {
      set({ snapshotLoading: false });
    }
  },
  settings: null,
  loadSettings: async () => {
    const settings = await api.getSettings();
    set({ settings });
    return settings;
  },
  setSettings: (settings) => set({ settings }),
  toast: null,
  showToast: (message, action) =>
    set({ toast: { id: nextToastId++, message, action } }),
  dismissToast: () => set({ toast: null }),
}));
