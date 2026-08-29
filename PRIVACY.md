# Privacy

Accountability OS is a personal productivity tool, not spyware. Monitoring never starts without
explicit consent (onboarding step 2), and the monitoring state (active / paused / demo /
permission required) is always visible in the sidebar and tray.

## Collected — locally, into SQLite on your machine

- Active application name and process name
- Active window title
- Start/end timestamps and durations
- System idle time and lock state
- Active browser tab **domain** and **title** — only if you install the companion extension
- Your commitments, plans, check-in and intervention answers
- The classification of each activity session

## Never collected

- Keystrokes or passwords (there is no keylogger of any kind)
- Clipboard contents
- Message, chat or email bodies
- Form contents
- Microphone or camera
- Raw browser history or background tabs
- Screen recordings or screenshots

## Where data lives

Everything is stored in a local SQLite database:
`%APPDATA%\com.accountability-os.desktop\accountability.sqlite3` (Windows).
Nothing is synced anywhere. There is no server.

## What reaches the AI provider (only if you enable AI)

For activity classification, only the minimum context needed to compare one activity with the
active commitment:

```
Active commitment: "Finish PA Commercial Sales Playbook"
Definition of done: …
Application: Chrome
Window: "Commercial Sales Playbook - Google Docs"
Domain: docs.google.com
Question: How related is this activity to the active commitment?
```

Private and excluded applications are never sent. Rules and your manual corrections are consulted
first, and answers are cached, so most sessions never reach the AI at all.

If you separately enable AI coaching and request it, the provider also receives the minimum
summary needed for that feature: proposed commitment titles plus aggregate completion patterns for
morning coaching; commitment titles/statuses, aggregate category minutes, scores and the top
distraction label for a daily review; or already-computed numeric pattern sentences for long-term
insights. It does not receive raw session history or database exports. AI can be disabled entirely;
the app remains fully functional.

## Controls you have

- **Excluded applications/domains** — never recorded at all (a hole in the timeline).
- **Private applications** — recorded only as "Private Application" + duration; no titles.
- **Pause monitoring** — one click in the tray or Settings.
- **Delete** today's activity, a date range, or all monitoring history and its derived AI/cache data
  (Settings → Your data).
- **Export** everything to JSON (Settings → Your data).
- **Retention** — activity older than the configured window is pruned automatically.

## Secrets

The AI API key is stored in the OS credential store (Windows Credential Manager), never in
SQLite, never in logs. The extension pairing token authenticates the local browser bridge and
never leaves your machine (it is excluded from data exports).
