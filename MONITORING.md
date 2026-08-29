# Monitoring

## What the probe reads (Windows)

`src-tauri/src/monitor/windows.rs`, polled every 3 seconds by the engine thread:

| Fact | Win32 API |
| --- | --- |
| Foreground window | `GetForegroundWindow` |
| Window title | `GetWindowTextW` |
| Owning process | `GetWindowThreadProcessId` → `OpenProcess` → `QueryFullProcessImageNameW` |
| Idle seconds | `GetLastInputInfo` vs `GetTickCount` |
| Lock state | `OpenInputDesktop(DESKTOP_SWITCHDESKTOP)` fails while locked |

The probe reports facts only. Classification is a separate service (see ARCHITECTURE.md).

## Aggregation (spec §8)

Polls are folded into **activity sessions** by `aos_core::aggregator`:

- Identity = process + browser domain + *normalized* window title. Cosmetic title changes
  (unread counters like `(3) `, unsaved markers) do not split a session; a different document does.
- **Idle** starts when input stopped, not when the threshold was crossed: at detection the previous
  session is truncated back to `now − idle_seconds`. Lock counts as idle immediately.
- A gap larger than 30 s (sleep, monitor paused) closes the old session at its last sighting
  instead of bridging the hole — unless the gap is explained by idleness itself.
- Sessions shorter than 5 s are dropped as switch noise.
- Result: "Chrome / Google Docs for 17 minutes" is **one row**, not 340.

All of this is unit-tested in `crates/aos-core/src/aggregator.rs`.

## Idle vs distraction

Idle is tracked separately and never counted as distraction (spec §10). The default idle
threshold is 3 minutes without input (configurable in Settings).

## Privacy filters (before anything is stored)

1. **Excluded** apps/domains → the sample is discarded entirely; the timeline shows a hole.
2. **Private** apps → recorded as "Private Application" + duration; no title, no process retained.

## Monitoring status (spec §41)

Always visible in the sidebar and tray: **Active**, **Paused**, **Demo simulation**, or
**Permission required** (with the reason and, where possible, a path to resolve it). It never
fails silently.

## Demo Mode (spec §47)

`monitor/demo.rs` replaces only the probe with a scripted loop — Google Docs focus, Gmail,
8 minutes of X (long enough to cross both the 3-minute warning and the 7-minute intervention),
more Docs, a real idle stretch, Slack. Everything downstream (aggregation, classification,
distraction tracking, scoring) is the production pipeline. `Settings → seed demo data` also
inserts the spec §48 sample day so the timeline and scorecard have history to show.

## Performance (spec §49)

- One poll every 3 s on a background thread; no per-poll DB writes — only session boundaries hit
  SQLite.
- Live classification is resolved from rules/corrections/cache in-memory; AI runs at most once per
  unique (commitment, app, domain, title) context, asynchronously.
- The frontend refreshes on events plus a 5 s snapshot poll; the widget ticks its clock locally.
