# Architecture

Local-first Tauri v2 desktop application. One Rust backend process owns monitoring, state and
the database; the React frontend renders windows and calls typed commands.

```
┌────────────────────────────────────────────────────────────────────┐
│ React/TS frontend (one bundle, four windows)                       │
│   main (Today/Tasks/Plan/Activity/Scorecard/Settings)              │
│   intervention popup · always-on-top widget · quick capture        │
└──────────────▲────────────────────────────┬────────────────────────┘
     typed invoke() commands          "app-event" channel (AppEvent)
┌──────────────┴────────────────────────────▼────────────────────────┐
│ src-tauri (Rust)                                                   │
│  commands/*        thin IPC layer, one wrapper per command         │
│  engine.rs         3s tick: probe → aggregate → classify → checks  │
│  monitor/          ActivityProbe: windows.rs (Win32) | demo | noop │
│  classify (core)   rules → corrections → cache → AI                │
│  ai.rs             OpenAI-compatible client, schema-validated      │
│  server.rs         127.0.0.1 bridge for the browser extension      │
│  tray.rs           system tray, quit-with-flush                    │
│  db/               rusqlite + migrations + repositories            │
└──────────────┬─────────────────────────────────────────────────────┘
┌──────────────▼──────────────┐   ┌─────────────────────────────────┐
│ crates/aos-core (pure Rust) │   │ extension/ (Chrome/Edge MV3)    │
│  aggregator · classify ·    │   │  active tab → POST /v1/activity │
│  scoring · accountability · │   │  localhost only, token auth     │
│  patterns · events          │   └─────────────────────────────────┘
└─────────────────────────────┘
```

## Separation of concerns (spec §32–34)

- **MonitoringService** (`monitor/` + the engine tick) detects foreground window, process, title,
  idle and lock. It never decides whether activity is productive.
- **ClassificationService** (`aos_core::classify` + `engine.rs` glue) applies deterministic rules,
  historical corrections, the cache, and finally AI. Confidence < 0.65 → `Unknown` (spec §12).
- **AccountabilityEngine** (`aos_core::accountability` + `engine.rs`) owns distraction thresholds,
  check-in scheduling, recovery tracking, breaks, strict mode, interview/review triggers. None of
  this logic lives in React components.
- **Scoring** (`aos_core::scoring`) is pure functions with unit tests; the daily score exposes all
  components (spec §20: show the math).

`aos-core` has zero Tauri/SQLite/OS dependencies, which is what makes the business rules testable
on any platform and is the path to macOS support: only `monitor/` needs a new probe.

## The engine tick (every 3 s)

1. Read the probe (real Win32 probe, or the demo script when Demo Mode is on).
2. Merge fresh extension metadata when a browser is foreground.
3. Drop excluded apps/domains; scrub private apps to "Private Application".
4. Feed the sample to the aggregator; a finished session is classified and stored
   (AI classifications happen async and patch the row when they land).
5. Classify the *open* activity for live UI + the distraction tracker.
6. Run accountability checks: break end, distraction warn/intervene/recovery, check-in due,
   morning interview due, end-of-day review due.
7. Emit `AppEvent`s; the UI refetches what changed.

## Events (spec §31)

`aos_core::events::AppEvent` is a tagged union emitted on a single `app-event` channel:
`ACTIVITY_CHANGED, FOCUS_STARTED/ENDED, COMMITMENT_CHANGED, DISTRACTION_WARNING/DETECTED/RESOLVED,
CHECKIN_DUE/ANSWERED, PRIORITY_CHANGE_REQUESTED, BLOCKED_FLOW_REQUESTED, BREAK_STARTED/ENDED,
TASK_COMPLETED, DAY_LOCKED/ENDED, INTERVIEW_DUE, REVIEW_DUE, MONITORING_STATUS,
SESSIONS_UPDATED, SCORES_UPDATED`.

## Threading model

- **engine thread** — the 3 s tick loop; owns the probes.
- **extension-bridge thread** — `tiny_http` listener on `127.0.0.1:<port>`.
- **tokio tasks** (Tauri async runtime) — AI calls; they update the DB/cache and emit events.
- Shared state: `AppState { db: Mutex<Connection>, engine: Mutex<EngineState>,
  activity_generation: AtomicU64, ai_key, http }`.
  The tray refresh never runs while the engine lock is held (deadlock rule).

## Windows

- `main` — the app. Closing hides to tray; monitoring continues (spec §56.16).
- `intervention` — always-on-top prompt window; renders whatever `get_pending_prompt` returns
  (intervention, check-in, break-over). Strict Mode keeps it open until answered.
- `widget` — optional always-on-top mini focus widget (spec §25).
- `capture` — quick capture, opened by the global `Ctrl+Shift+Space` shortcut.
