// Accountability OS Companion — background service worker.
//
// Privacy contract (spec §9):
// - Only the ACTIVE tab of a FOCUSED browser window is ever reported.
// - Reports go only to the local desktop app at 127.0.0.1.
// - No history, no background tabs, no page content.

const DEFAULTS = { port: 43117, token: "" };

async function getConfig() {
  const stored = await chrome.storage.local.get(DEFAULTS);
  return { port: stored.port || 43117, token: stored.token || "" };
}

function domainOf(url) {
  try {
    const u = new URL(url);
    if (u.protocol !== "http:" && u.protocol !== "https:") return null;
    return u.hostname;
  } catch {
    return null;
  }
}

async function reportActiveTab(windowFocused) {
  const config = await getConfig();
  if (!config.token) return; // not paired yet

  // Event callbacks can fire for an active tab in a background browser
  // window (for example when its title changes). Never trust the callback's
  // implied focus state; verify it immediately before reading tab metadata.
  const focusedWindow = windowFocused ? await getFocusedWindow() : null;
  windowFocused = Boolean(focusedWindow);

  let payload = { domain: "", title: "", window_focused: false };
  if (windowFocused) {
    const [tab] = await chrome.tabs.query({ active: true, windowId: focusedWindow.id });
    if (!tab || !tab.url) return;
    // Bind the report to the same focused window after the asynchronous tab
    // lookup too, closing the small focus-change race between both calls.
    const stillFocused = await getFocusedWindow();
    if (!stillFocused || stillFocused.id !== focusedWindow.id) return;
    const domain = domainOf(tab.url);
    if (!domain) return; // chrome:// pages etc. — report nothing
    payload = {
      domain,
      title: (tab.title || "").slice(0, 300),
      window_focused: true,
    };
  }

  try {
    await fetch(`http://127.0.0.1:${config.port}/v1/activity`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-AOS-Token": config.token,
      },
      body: JSON.stringify(payload),
    });
  } catch {
    // Desktop app not running — silently idle.
  }
}

async function isAnyWindowFocused() {
  return Boolean(await getFocusedWindow());
}

async function getFocusedWindow() {
  try {
    const win = await chrome.windows.getLastFocused();
    return win && win.focused && Number.isInteger(win.id) ? win : null;
  } catch {
    return null;
  }
}

chrome.tabs.onActivated.addListener(() => void reportActiveTab(true));

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  // Only the active tab's committed navigation / title changes matter.
  if (tab.active && (changeInfo.title || changeInfo.url || changeInfo.status === "complete")) {
    void reportActiveTab(true);
  }
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  void reportActiveTab(windowId !== chrome.windows.WINDOW_ID_NONE);
});

// Freshness heartbeat: the desktop app treats reports older than 60s as
// stale, so re-report every 30s while a browser window has focus.
chrome.alarms.create("heartbeat", { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name === "heartbeat") {
    void reportActiveTab(await isAnyWindowFocused());
  }
});

chrome.runtime.onStartup.addListener(async () => {
  void reportActiveTab(await isAnyWindowFocused());
});
