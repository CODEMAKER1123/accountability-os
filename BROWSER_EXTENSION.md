# Browser Extension

The Chrome/Edge MV3 companion (`extension/`) adds precision to browser activity: without it the
app only sees window titles; with it, the active tab's **domain** and **title** — nothing more.

## Privacy contract (spec §9)

- Reports **only the active tab**, and only while a browser window is **focused**.
- Sends **only** to `http://127.0.0.1:<port>` — the desktop app on the same machine.
- No history, no background tabs, no page content, no cloud.
- `chrome://` and other non-http(s) pages are never reported.
- Requests authenticate with a pairing token; the desktop app rejects anything else.

## Install (Chrome or Edge)

1. Open `chrome://extensions` (or `edge://extensions`).
2. Enable **Developer mode**.
3. **Load unpacked** → select the `extension/` folder.
4. Open the extension's **Options** page.
5. In the desktop app: Settings → Browser extension → **Reveal** the pairing token, copy it.
6. Paste the token (and port, default `43117`) into the options page → **Save** →
   **Test connection** should report "Connected to Accountability OS".

The desktop app's Settings page shows **Connected** once reports arrive (heartbeat every 30 s
while the browser is focused; the app treats reports older than 60 s as stale).

## How it works

- `background.js` listens to `tabs.onActivated`, `tabs.onUpdated` (active tab only) and
  `windows.onFocusChanged`, plus a 30 s freshness heartbeat via `chrome.alarms`.
- Each report: `POST /v1/activity {domain, title, window_focused}` with the `X-AOS-Token` header.
- The desktop bridge (`src-tauri/src/server.rs`, bound to `127.0.0.1` only) stores the latest
  report in memory; the engine merges it into browser sessions while a browser process is
  foreground and *Browser monitoring* is enabled in Settings.

Changing the port requires updating both the desktop Settings and the extension options, then
restarting the desktop app (the bridge binds at startup).
