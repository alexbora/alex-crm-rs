# Copilot instructions for `alex-crm-rs`

Start with `CRM_RUST_sisyphus.md`. It is the repository's single source of truth for the current product definition, architecture, defaults, and change-governance notes, and it is expected to contain both instructions and explanations. If a change affects architecture or behavior, update that file in the same change.

## Build, test, and lint commands

This repository uses plain Cargo commands; there is no repo-specific wrapper script.

```powershell
cargo run
cargo build
cargo build --release
cargo test
```

Single-test pattern:

```powershell
cargo test no_retry_calls_once -- --exact
```

Current unit tests live in `src/policies/retry.rs`, so name filtering is the practical way to run one test at a time.

Lint / formatting commands:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

Important Windows build detail from `.cargo/config.toml`:

- the default target is `x86_64-pc-windows-gnu`
- `CMAKE_GENERATOR` is set to `MinGW Makefiles`
- the config also injects GNU-style link args, so avoid switching the project to the MSVC target unless you are intentionally changing the toolchain story

## High-level architecture

`src/main.rs` is the composition root. It creates:

- one bounded UI -> DB queue: `DbTask` (capacity 256)
- one bounded workers -> UI queue: `UiTask` (capacity 256)
- one bounded DB -> backup control queue: `BackupCommand` (capacity 32)
- one `DbWorker` thread
- one `BackupWorker` thread
- the FLTK UI loop through `App<L, R>`

The application is intentionally split by thread ownership:

- `src/app.rs` owns the FLTK event loop, builds the main window, triggers the initial company fetch and today's log fetch, and drains `UiTask` messages back into widgets
- `src/backend/db_worker.rs` owns the single SQLite connection, bootstraps schema + pragmas, performs all CRUD work, and forwards manual backup requests to the backup worker
- `src/backend/backup_worker.rs` owns scheduled/manual file-copy backups and reports backup status back through `UiTask`
- `src/ui/*.rs` builds the windows and sends typed tasks only; it does not touch SQLite directly

The schema in `src/backend/schema.rs` is more important than the README suggests:

- core tables: `companies`, `counties`, `contacts`, `company_contacts`, `activities`, `daily_logs`
- company search uses both case-insensitive `LIKE` and SQLite FTS5 via `companies_fts`
- FTS maintenance is done with triggers, so schema changes around company names need matching trigger/FTS updates
- the DB runs with `foreign_keys = ON`, `journal_mode = WAL`, `synchronous = NORMAL`, and `busy_timeout = 5000`

The UI currently has four tabs, but only two are materially wired:

- `Companie`: active CRUD/search flow for companies
- `Logs`: active daily log save/reload and backup request flow
- `Contacts` and `Activities`: placeholder tabs in `src/ui/main_window.rs`, not backend-complete yet

## Key conventions

- Keep all cross-thread communication inside the task enums in `src/tasks.rs`. New backend capabilities should usually mean adding a `DbTask` variant and a matching `UiTask` result rather than calling into the DB from UI code.
- Preserve the policy-injection pattern. Logging and retry are traits in `src/policies/`, and `App`, `DbWorker`, and `BackupWorker` are generic over `LoggingPolicy` / `RetryPolicy`. Reuse those traits instead of hardcoding logging or retry behavior into business logic.
- The current default composition in `main.rs` is `ConsoleLogger` + `ExponentialBackoff`. Treat that file as the place where runtime wiring decisions belong.
- Mutating DB operations in `DbWorker` use explicit `BEGIN IMMEDIATE` transactions with manual rollback paths and return `OperationResult { success, message }` for user-facing outcomes. Follow that pattern for new write operations.
- Search semantics are intentionally two-layered: raw text search still uses `LIKE`, but `build_fts_query()` also sanitizes each token down to ASCII alphanumeric / underscore and appends `*` for prefix FTS matching. Reuse that behavior instead of inventing a separate search format.
- The database paths are fixed in `main.rs` under `data\notes_app.db` and `data\notes_app_backup.db`. Backup logic assumes file-based copies between those locations.
- Opening a company detail window also creates and opens a relative folder path `CompanyName\YYYY\MM`. The helper sanitizes Windows-invalid filename characters to `_`; preserve that behavior if you touch detail-window flow.
- `Companie` is intentionally spelled that way in the UI and is documented as a deliberate default in `CRM_RUST_sisyphus.md`; do not "fix" it casually.
- `src/ui/detail_window.rs` enforces one detail window per company ID via `DetailWindowStore`. Reuse that store instead of opening duplicate edit windows.
- `src/ui/companies_tab.rs` keeps opened new-company windows in a retained vector. If you change that lifecycle, check `CRM_RUST_sisyphus.md` first because window-retention hygiene is already called out there as a known improvement.
