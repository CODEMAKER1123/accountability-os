# Development

## Prerequisites

- **Node 22+** and npm
- **Rust** (stable, 1.80+)
- Windows: the [Tauri Windows prerequisites](https://tauri.app/start/prerequisites/) (WebView2 is
  preinstalled on Windows 11; MSVC Build Tools required)
- Linux (for development/CI): `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
  librsvg2-dev libglib2.0-dev pkg-config`

## Install & run

```bash
npm install          # frontend deps (also installs @tauri-apps/cli)
npm run tauri dev    # dev build with hot reload (starts Vite on :1420)
```

On first launch the onboarding runs: consent, work hours, strictness, optional AI, extension,
a live monitoring test, first task. On non-Windows dev machines enable **Demo Mode** in the
monitoring test step — the native probe is Windows-only, the simulator drives everything else.

## Tests & checks

```bash
npm run typecheck        # tsc --noEmit (strict)
npm test                 # vitest
cargo test --workspace   # 62 Rust tests: aggregation, idle, classification,
                         # scoring, check-ins, thresholds, breaks, switching,
                         # daily score, DB persistence, AI response validation
cargo check --workspace  # full compile check
```

The important business logic is tested in `crates/aos-core` (no UI, no OS deps) plus DB
round-trip tests in `src-tauri/src/db/mod.rs`.

## Build the Windows installer

On Windows:

```bash
npm install
npx tauri build
# → target/release/bundle/nsis/Accountability OS_<version>_x64-setup.exe
```

Or let CI do it: the **Windows installer** GitHub Actions workflow builds the NSIS installer on
every push and uploads it as the `accountability-os-windows-installer` artifact.

## Configure AI

Settings → AI: base URL, models, API key (stored in Windows Credential Manager), then
**Test connection** — it runs a real classification round-trip. Details in [AI.md](AI.md).

## Install the browser extension

See [BROWSER_EXTENSION.md](BROWSER_EXTENSION.md).

## Reset local data

Close the app (tray → Quit), then delete the data directory:

- Windows: `%APPDATA%\com.accountability-os.desktop\`
- Linux (dev): `~/.local/share/com.accountability-os.desktop/`

Inside the app: Settings → Your data offers *Delete today / delete all monitoring history /
export JSON* without touching tasks and plans.

To remove the stored AI key as well: Settings → AI → save an empty key (deletes the credential).

## Project layout

See [ARCHITECTURE.md](ARCHITECTURE.md). Quick map:

- `crates/aos-core/` — pure domain logic + the bulk of the tests. Start here for behavior changes.
- `src-tauri/src/engine.rs` — the 3-second tick that ties everything together.
- `src-tauri/src/commands/` — the IPC surface; mirrored 1:1 by `src/lib/ipc.ts`.
- `src/views/`, `src/components/`, `src/windows/` — the UI.

## Conventions

- TypeScript strict; every backend command has exactly one typed wrapper in `src/lib/ipc.ts`.
- Rust: business rules go in `aos-core` (testable), OS/IO in `src-tauri`.
- DB schema changes = append a migration (see [DATABASE.md](DATABASE.md)).
- Never hold the `engine` mutex while calling something that takes it (tray refresh, snapshot).
