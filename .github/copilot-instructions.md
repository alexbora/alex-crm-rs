# Copilot instructions for `alex-crm-rs`

## Start Here
- `CRM_RUST_sisyphus.md` is the single source of truth for product definition, architecture, and conventions. Update it with any structural or behavioral change.

## Build, Test, and Lint
- Use standard Cargo commands:
  ```powershell
  cargo run
  cargo build
  cargo build --release
  cargo test
  ```
- Run a single test (unit tests are in `src/policies/retry.rs`):
  ```powershell
  cargo test <test_name> -- --exact
  ```
- Lint/format:
  ```powershell
  cargo fmt --all --check
  cargo clippy --all-targets --all-features -- -D warnings
  ```
- Windows: Default target is `x86_64-pc-windows-gnu` with `CMAKE_GENERATOR=MinGW Makefiles` (see `.cargo/config.toml`). Avoid switching to MSVC unless intentionally changing toolchain.

## High-Level Architecture
- `src/main.rs` is the composition root. It spawns:
  - FLTK UI thread (`App<L, R>`, see `src/app.rs`)
  - DB worker thread (`DbWorker`, see `src/backend/db_worker.rs`)
  - Backup worker thread (`BackupWorker`, see `src/backend/backup_worker.rs`)
- Communication is via bounded `crossbeam::channel` queues:
  - UI → DB: `DbTask` (256)
  - Workers → UI: `UiTask` (256)
  - DB → Backup: `BackupCommand` (32)
- Policy injection: Logging and retry are traits, injected at runtime (`App<L, R>`). Default: `ConsoleLogger` + `ExponentialBackoff`.
- UI tabs: `Companie` (intentional spelling), `Contacts`, `Activities`, `Logs`. Only `Companie` and `Logs` are fully wired.
- Company detail opens folder `CompanyName/YYYY/MM` (invalid filename chars replaced with `_`).
- Database: SQLite, FTS5 for company search, WAL mode, triggers for FTS maintenance. Paths: `data/notes_app.db`, `data/notes_app_backup.db`.

## Key Conventions
- All cross-thread work must use task enums in `src/tasks.rs` (`DbTask`, `UiTask`, `BackupCommand`).
- Never call the DB directly from UI code; always use the task queue.
- Mutating DB ops use explicit `BEGIN IMMEDIATE` transactions and return `OperationResult { success, message }`.
- Search uses both `LIKE` and FTS5; FTS queries are sanitized and use prefix matching.
- Always update `CRM_RUST_sisyphus.md` for any change in structure or behavior.
- The `Companie` tab label is intentional—do not "fix" it.
- Only one detail window per company ID (`DetailWindowStore`).
- New-company windows are retained in a vector; see `CRM_RUST_sisyphus.md` for lifecycle notes.
- If code and `CRM_RUST_sisyphus.md` diverge, update both in the same change.

---

This file summarizes the essential build, architecture, and convention details for Copilot and other AI agents. Let me know if you want to adjust anything or add coverage for other areas.
