// App-level state for the main window: navigation, snapshot, settings, modals.

import { create } from "zustand";

import { api, type Settings, type TodaySnapshot } from "./ipc";

export type View = "today" | "tasks" | "plan" | "activity" | "scorecard" | "settings";

export type Modal =
  | { kind: "interview" }
  | { kind: "review" }
  | { kind: "switch"; fromCommitmentId: number | null; toCommitmentId?: number | null }
  | { kind: "blocked"; commitmentId: number }
  | { kind: "break" }
  | { kind: "onboarding" }
  | null;

interface Store {
  view: View;
  setView: (v: View) => void;
  modal: Modal;
  setModal: (m: Modal) => void;
  snapshot: TodaySnapshot | null;
  refreshSnapshot: () => Promise<void>;
  settings: Settings | null;
  loadSettings: () => Promise<Settings>;
  setSettings: (s: Settings) => void;
}

export const useStore = create<Store>((set) => ({
  view: "today",
  setView: (view) => set({ view }),
  modal: null,
  setModal: (modal) => set({ modal }),
  snapshot: null,
  refreshSnapshot: async () => {
    try {
      const snapshot = await api.getTodaySnapshot();
      set({ snapshot });
    } catch (e) {
      console.error("snapshot refresh failed", e);
    }
  },
  settings: null,
  loadSettings: async () => {
    const settings = await api.getSettings();
    set({ settings });
    return settings;
  },
  setSettings: (settings) => set({ settings }),
}));
