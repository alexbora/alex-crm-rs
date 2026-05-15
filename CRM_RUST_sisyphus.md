# Alex CRM (Rust) — Single Source of Truth

This document is the **only authoritative project spec** for `alex-crm-rs`.
It replaces older duplicated plans and conflicting notes.

## 1. Interpreted Product Definition

The intended app is interpreted as:

1. **Native Rust desktop CRM** using **FLTK** for UI and **SQLite** for storage.
2. UI and backend run on separate threads and communicate only through typed task queues.
3. Policies (logging/retry) are static, compile-time style (`App<L, R>`), matching the C++ template intent.
4. Main UI has 4 tabs: **Companie**, **Contacts**, **Activities**, **Logs**.
5. Clicking a company opens detail/edit UI and opens folder path `CompanyName/YYYY/MM`.
6. Daily logs are saved in SQLite (one entry per day via upsert).

> Note: Earlier references to Tauri/webview flow are considered out-of-scope for this repository.

---

## 2. Canonical Current Code Layout

```text
src/
├── main.rs
├── app.rs
├── state.rs
├── tasks.rs
├── backend/
│   ├── mod.rs
│   ├── schema.rs
│   ├── db_worker.rs
│   └── backup_worker.rs
├── policies/
│   ├── mod.rs
│   ├── logging.rs
│   └── retry.rs
└── ui/
    ├── mod.rs
    ├── main_window.rs
    ├── companies_tab.rs
    ├── detail_window.rs
    └── new_company_form.rs
```

Data paths:
- primary DB: `data/notes_app.db`
- backup DB: `data/notes_app_backup.db`

---

## 3. Architecture (Current)

```text
main()
  ├─ spawn backup worker thread
  ├─ spawn DB worker thread
  └─ run FLTK app loop (UI thread)
```

Message passing:
- UI -> DB: `crossbeam::channel::bounded<DbTask>(256)`
- Workers -> UI: `crossbeam::channel::bounded<UiTask>(256)`
- DB -> Backup control: `crossbeam::channel::bounded<BackupCommand>(32)`

Policies:
- Logging: `NoLogging | ConsoleLogger | FileLogger`
- Retry: `NoRetry | SpinRetry | ExponentialBackoff`
- Composition root: `App<L: LoggingPolicy, R: RetryPolicy>`

---

## 4. Requirement Coverage

| Requirement | Status | Implementation |
|---|---|---|
| FLTK UI + SQLite backend | Done | `ui/*`, `backend/db_worker.rs`, `backend/schema.rs` |
| Thread separation (UI/backend) | Done | worker threads in `main.rs` |
| Queue/task-based interaction only | Done | `DbTask` / `UiTask` enums in `tasks.rs` |
| Logging policy variants | Done | `policies/logging.rs` |
| Retry/backoff policy variants | Done | `policies/retry.rs` |
| 4 tabs (Companie/Contacts/Activities/Logs) | Done | `ui/main_window.rs` |
| Companies list from `notes_app.db` | Done | fetch flow in `db_worker.rs` + companies tab |
| Company details/edit on record open | Done | `ui/detail_window.rs` + update task |
| Open folder `CompanyName/YYYY/MM` | Done | `open_company_folder` in `ui/detail_window.rs` |
| Daily logs input/storage | Done | logs UI + `daily_logs` table read/write |
| Backup capability | Done | `backend/backup_worker.rs` + request flow |

---

## 5. Deliberate Defaults

- DB queue capacity: `256`
- UI queue capacity: `256`
- Backup control queue capacity: `32`
- Backup interval: `300` seconds
- FTS tokenizer: `porter`
- First tab label: `Companie` (kept intentionally)
- Folder root for company path: current working directory

---

## 6. Known Improvements (Next)

These are not blockers for current behavior but should be addressed next:

1. **True strict SPSC interpretation**: split worker->UI queue if strict one-producer-per-queue is required.
2. **Backup robustness**: switch from file-copy backup to SQLite backup API for safer hot backups.
3. **UI window lifecycle hygiene**: prune hidden new-company form windows from retained vector.
4. **Validation pass**: run `cargo build` and `cargo test` in a Rust-enabled environment and resolve any compile/runtime issues.

---

## 7. Build/Run Commands

```powershell
cd C:\Users\a049689\dev\alex-crm-rs
cargo build
cargo run
cargo test
cargo build --release
```

---

## 8. Change Governance

1. Any structural or behavioral decision must be updated here first.
2. If code and this file diverge, update this file in the same change set.
3. Old plan files are historical only; this file defines the active truth.

---

## 9. Code Audit Findings (May 2026)

### **Core Strengths**
*   **Architecture:** Clean **Worker Thread Pattern** decoupling UI (FLTK), Database (rusqlite), and Backup logic via `crossbeam` channels. UI responsiveness is maintained via non-blocking polling.
*   **Data Strategy:** Advanced SQLite usage including **FTS5 Full-Text Search**, WAL mode, and triggers for automated index maintenance.
*   **Patterns:** Robust **Policy Injection** (Traits) for logging and retries, following "Composition Root" principles.

### **Technical Observations**
*   **Modularity:** Strong separation between UI components and backend workers.
*   **Reliability:** Mature **Exponential Backoff** implementation for fallible operations.
*   **Lifecycle:** Explicit shutdown orchestration for clean resource release.

### **Identified Refinements**
*   **Error Handling:** Move from `String` errors to typed enums (e.g., `thiserror`) for better diagnostic precision.
*   **Configuration:** Externalize hardcoded database paths and backup intervals.
*   **Testing:** Expand from policy unit tests to `DbWorker` integration tests.

---

## 10. Automation Audit (May 2026)

### **Current Surface**
- **Scheduled Database Backup:** Background thread (`BackupWorker`) performs file-copy backups every 300 seconds.
- **Manual Backup Trigger:** UI-driven request to the backup worker via `BackupCommand::RunNow`.
- **Build & Test Cycle:** Standard `cargo` toolchain orchestration (Build/Run/Test).
- **Dynamic Policy Injection:** Trait-based logging and retry logic injected at the composition root (`App<L, R>`).

### **Key Findings**
- **Stale Documentation:** `README.md` overlaps with this document, creating potential for truth drift.
- **Missing CI/CD:** No automated verification (GitHub Actions) for build or test health.
- **Missing Hooks:** No pre-commit automation for code formatting (`cargo fmt`) or linting.
- **Ambiguous Validation:** Build/test status in the current environment is not recently verified by an agent.

### **Recommendations**
- **Keep:** `BackupWorker` and Policy Injection architecture.
- **Merge:** Consolidate any unique `README.md` content into this document; repurpose README as a high-level entry point.
- **Cut:** Deprecate `plan-archive/` to prevent agent confusion.
- **Fix Next:** 
    1. **Verification Loop:** Run `cargo build` and `cargo test` to prove current project health.
    2. **Infrastructure:** Create `.github/workflows/rust.yml` for automated CI.
    3. **Hooks:** Implement local pre-commit hooks for style consistency.

