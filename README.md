# Accountability OS

**AI desktop productivity & focus coach for Windows 11.**

This is not a to-do app. Accountability OS exists to answer one question:

> Did you actually spend your day doing what you said was important?

The core loop: **Plan → Commit → Monitor → Interrupt → Score → Review → Improve.**

Every morning it interviews you and locks a contract of 1–3 outcomes. It then watches which
applications and windows you actually use, classifies each activity against your active
commitment (rules → your corrections → AI), interrupts you when you drift, checks in
periodically, and closes the day with an execution score whose math is never hidden.

## What it does

- **Task backlog** — inbox/planned/committed/active/completed/deferred/cancelled, must/should/could
  priorities, projects, tags, estimates. Quick capture from anywhere with `Ctrl+Shift+Space`.
- **Morning interview** — seven questions ending in a locked daily contract ("LOCK MY DAY"). More
  than three priorities gets pushback: *six priorities means you have no priorities.* Selected
  outcomes can be split by AI into a simple, standard, or detailed editable checklist.
- **Desktop-wide monitoring** — native Win32 foreground/idle/lock probe, aggregated into activity
  sessions (one row per continuous activity, not hundreds of polls).
- **Hybrid classification** — deterministic rules first, your past corrections second, AI third,
  cached aggressively. Low-confidence answers become *Unknown*, never punishment.
- **Distraction detection** — 3 minutes off-plan → warning; 7 → an intervention window: *YOU'RE OFF
  PLAN.* Return to task / actually work / planned break / priority changed / blocked. Recovery time
  is measured.
- **Accountability check-ins** — every 90 minutes (configurable): *are you still working on the
  right thing?* with the actual numbers for the window.
- **Planned breaks** — never counted as distraction.
- **Daily review & score** — completion (40%) + alignment (30%) + focus quality (20%) + planning
  accuracy (10%). Component scores always shown.
- **Long-term patterns** — most productive hours, top distractions, estimation bias, deep-work
  blocks, completion by start time — deterministic numbers, optionally narrated by AI.
- **Strict Mode** — prompts stay until answered, snoozing is limited, switching requires a reason.
  Persistent, never hostile: it never locks your computer.
- **Local-first** — everything lives in a local SQLite database. See [PRIVACY.md](PRIVACY.md).
- **Works without AI** — rules, corrections, timers, scoring and reviews are fully functional
  offline; AI adds classification of ambiguous activity and coaching.

## The pieces

| Path | What it is |
| --- | --- |
| `src/` | React + TypeScript + Tailwind frontend (main window, prompt window, widget, quick capture) |
| `src-tauri/` | Tauri v2 desktop app: monitoring, accountability engine, SQLite, AI, tray |
| `crates/aos-core/` | Pure-Rust domain logic: aggregation, classification, scoring, thresholds — fully unit-tested |
| `extension/` | Chrome/Edge MV3 companion: active-tab domain/title → localhost only |
| `.github/workflows/` | CI + Windows NSIS installer build |

## Quick start

```bash
npm install
npm run tauri dev      # run the desktop app in dev mode
npm test               # frontend tests
cargo test --workspace # Rust tests
npx tauri build        # Windows installer (on Windows)
```

Full instructions: [DEVELOPMENT.md](DEVELOPMENT.md). Every push also builds an installable
Windows NSIS installer as a GitHub Actions artifact (`Windows installer` workflow).

No Windows machine handy? Enable **Demo Mode** in Settings (or during onboarding) — a scripted
probe simulates focused work, an 8-minute distraction and idle time through the entire real
pipeline, on any OS.

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — services, events, data flow
- [PRIVACY.md](PRIVACY.md) — exactly what is and is not collected
- [DATABASE.md](DATABASE.md) — SQLite schema
- [MONITORING.md](MONITORING.md) — the probe, aggregation, idle/lock semantics
- [AI.md](AI.md) — provider abstraction, payloads, cost control
- [BROWSER_EXTENSION.md](BROWSER_EXTENSION.md) — install & pairing
- [DEVELOPMENT.md](DEVELOPMENT.md) — build, test, reset, troubleshoot
