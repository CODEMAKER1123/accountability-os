# Database

SQLite via `rusqlite` (bundled), WAL mode, foreign keys on. Migrations are embedded in
`src-tauri/src/db/migrations.rs` and applied through `PRAGMA user_version`.

Location: `%APPDATA%\com.accountability-os.desktop\accountability.sqlite3`.

## Tables

| Table | Purpose |
| --- | --- |
| `settings` | Key/value; the app settings JSON blob lives under `app_settings`. The AI key does NOT live here (OS credential store). |
| `projects` | Task grouping. |
| `tasks` | Backlog: title, description, project, parent task, status (`inbox/planned/committed/active/completed/deferred/cancelled`), priority (`must/should/could`), estimate, due date, tags (JSON), timestamps. |
| `daily_plans` | One per date: lock time, likely distraction + countermeasure, when-most-important, raw interview answers, day-off flag, end time. |
| `daily_commitments` | 1–3 per locked plan: title, definition of DONE, estimate, priority, rank, status, outcome reason/note, and ordered action-step checklist (JSON). |
| `activity_sessions` | Aggregated monitoring output (spec §30): local_date, start/end, duration, app, process, window title, browser domain/title, classification (+confidence/source/reason), related commitment, idle flag, pending-AI flag. |
| `activity_corrections` | Every manual reclassification, with its context — feeds the correction matcher (layer 2). |
| `domain_rules` / `application_rules` | Deterministic layer-1 rules; default blocked domains are seeded with `is_default=1`. |
| `classification_cache` | AI answers keyed by `commitment|process|domain|normalized-title` (spec §33). |
| `focus_sessions` | Start/end/outcome of working on a commitment. |
| `checkins` + `checkin_responses` | Periodic accountability checks with the stats window shown, and the answer. |
| `interruptions` | Warnings, interventions, and priority switches: distracted seconds, response, recovery time. |
| `breaks` | Planned breaks: planned vs actual end. |
| `daily_reviews` | Review record per plan + optional AI summary. |
| `daily_scores` | Finalized per-date scores with all components and the raw second totals. |
| `ai_insights` | Deterministic and AI-narrated pattern insights per period (7d/30d/90d). |

## Conventions

- Timestamps are unix seconds (UTC); `local_date` / plan `date` are local `YYYY-MM-DD` strings so
  day queries follow the user's clock.
- Enum-ish columns are TEXT validated at the repository layer (`src-tauri/src/db/*.rs`).
- Deleting activity (today / range / all) removes matching sessions, corrections, exact
  classification-cache entries, check-ins, interruptions and scores; clears affected AI review
  text; and invalidates aggregate insights. Plans, commitments and tasks remain.

## Adding a migration

Append a new SQL string to `MIGRATIONS` in `migrations.rs`. Never edit an existing entry —
`user_version` gates execution, so shipped databases only run what they haven't seen.
