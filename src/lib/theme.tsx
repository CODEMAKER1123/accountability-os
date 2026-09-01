import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type Theme = "light" | "dark";

export const THEME_STORAGE_KEY = "accountability-os.theme";
const THEME_CHANNEL_NAME = "accountability-os-theme";

export function parseTheme(value: string | null | undefined): Theme {
  return value === "light" ? "light" : "dark";
}

export function readStoredTheme(): Theme {
  try {
    return parseTheme(window.localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return "dark";
  }
}

export function applyDocumentTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
}

interface ThemeContextValue {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => readStoredTheme());

  useEffect(() => {
    applyDocumentTheme(theme);
    if ("__TAURI_INTERNALS__" in window) {
      void getCurrentWindow()
        .setTheme(theme)
        .catch(() => {
          // Native title-bar theming is best effort in browser previews and
          // older Tauri runtimes.
        });
    }
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // Storage can be unavailable in a restricted browser preview.
    }
  }, [theme]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === THEME_STORAGE_KEY && event.newValue != null) {
        setThemeState(parseTheme(event.newValue));
      }
    };
    window.addEventListener("storage", onStorage);

    let channel: BroadcastChannel | null = null;
    if (typeof window.BroadcastChannel !== "undefined") {
      channel = new window.BroadcastChannel(THEME_CHANNEL_NAME);
      channel.onmessage = (event: MessageEvent<unknown>) => {
        if (typeof event.data === "string") setThemeState(parseTheme(event.data));
      };
    }

    return () => {
      window.removeEventListener("storage", onStorage);
      channel?.close();
    };
  }, []);

  const setTheme = useCallback((nextTheme: Theme) => {
    setThemeState(nextTheme);
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, nextTheme);
      if (typeof window.BroadcastChannel !== "undefined") {
        const channel = new window.BroadcastChannel(THEME_CHANNEL_NAME);
        channel.postMessage(nextTheme);
        channel.close();
      }
    } catch {
      // Keep the in-memory selection even when cross-window sync is unavailable.
    }
  }, []);

  const value = useMemo(() => ({ theme, setTheme }), [theme, setTheme]);
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useTheme must be used inside ThemeProvider");
  return value;
}
