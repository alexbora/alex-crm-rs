# Alex CRM — Rust + FLTK Development Plan

> **Stack:** Rust · FLTK-rs (native GUI) · SQLite3 FTS5 (DB worker thread)  
> **Build:** Cargo only · Windows/macOS/Linux · no npm, no Node.js  
> **Architecture:** Same as C++ version but in Rust — tokio async or `std::thread`, crossbeam SPSC, trait-based policies

---

## 0 — Architecture Overview

```
main()
  └─ App<ConsoleLogger, ExponentialBackoff>
       ├─ FLTK UI thread (event loop)
       │    └─ crossbeam::channel::unbounded() ← backend posts results
       └─ Tokio backend (DB + backup workers)
            └─ crossbeam::channel::bounded(256)  ← UI posts requests
```

**Key principle:** UI thread and backend are separated by a single-producer-single-consumer channel. No shared mutexes, only message passing.

---

## Phase 0 — Setup & Toolchain

### Task 0.1 — Install Rust (if needed)

```bash
# One-time setup (Windows PowerShell as Admin)
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe
.\rustup-init.exe -y

# Verify
rustc --version
cargo --version
```

**No npm, no Node.js required.**

---

### Task 0.2 — Initialize Cargo Project

```bash
cd C:\Users\a049689\dev\alex-crm-rs
cargo init --name alex-crm
```

Creates:
```
src/
  └── main.rs
Cargo.toml
Cargo.lock
.gitignore
```

---

### Task 0.3 — Project Structure

```
alex-crm-rs/
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── main.rs                (FLTK app entry + Tokio runtime setup)
│   ├── app.rs                 (App<LoggingPolicy, RetryPolicy>)
│   ├── backend/
│   │   ├── mod.rs
│   │   ├── db_worker.rs       (SQLite worker, processes DbTask)
│   │   ├── backup_worker.rs   (scheduled backup)
│   │   └── schema.rs          (schema bootstrap, FTS5 setup)
│   ├── policies/
│   │   ├── mod.rs
│   │   ├── logging.rs         (NoLogging, ConsoleLogger, FileLogger)
│   │   └── retry.rs           (NoRetry, SpinRetry, ExponentialBackoff)
│   ├── tasks.rs               (DbTask, UiTask, request/response types)
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── main_window.rs     (FLTK main window + tabs)
│   │   ├── companies_tab.rs   (company browser + search)
│   │   ├── detail_window.rs   (company detail editor)
│   │   └── new_company_form.rs (new company dialog)
│   └── state.rs               (shared state, channels, logger)
└── data/
    └── notes_app.db          (created at runtime)
```

---

## Phase 1 — Foundation & Infrastructure (Rust)

### Task 1.1 — Message Types (`src/tasks.rs`)

**Goal:** Serializable request/response structs for cross-thread communication.

```rust
// Comment: All types sent between UI thread and DB worker thread
// Producer: UI thread sends DbTask to backend via db_tx channel
// Producer: Backend sends UiTask back to UI via ui_tx channel

pub struct CompanyRow {
    pub id: i64,
    pub name: String,
}

pub struct FetchCompaniesReq {
    pub search: String,
    pub offset: u32,
    pub limit: u32,
}

pub struct FetchCompaniesResult {
    pub rows: Vec<CompanyRow>,
    pub total: u32,
}

pub struct InsertCompanyReq {
    pub name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

pub enum UiTask {
    FetchCompaniesResult(Result<FetchCompaniesResult, String>),
    InsertCompanyResult(Result<OperationResult, String>),
    UpdateCompanyResult(Result<OperationResult, String>),
    DeleteCompanyResult(Result<OperationResult, String>),
    BackupStatusResult(String),
}

pub enum DbTask {
    FetchCompanies(FetchCompaniesReq),
    InsertCompany(InsertCompanyReq),
    UpdateCompany(i64, String),  // (id, new_name)
    DeleteCompany(i64),
    RequestBackup,
    Shutdown,  // Sentinel
}
```

**Acceptance:** File compiles with `cargo build`.

---

### Task 1.2 — Logging Policy Trait (`src/policies/logging.rs`)

**Goal:** Three compile-time logging policies selected via trait objects.

```rust
pub trait LoggingPolicy: Send + Sync {
    fn log(&self, level: &str, message: &str);
    fn flush(&self);
    fn is_enabled(&self) -> bool;
}

// NoLogging — zero-cost no-op
// ConsoleLogger — stderr with timestamp
// FileLogger — daily rotating file in logs/YYYY-MM-DD.log
```

**Acceptance:** All three implementations compile and are thread-safe.

---

### Task 1.3 — Retry/Backoff Policy Trait (`src/policies/retry.rs`)

**Goal:** Compile-time retry strategies.

```rust
pub trait RetryPolicy: Send + Sync {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>;
}

// NoRetry — immediate result
// SpinRetry<N> — spin-lock N times
// ExponentialBackoff — sleep doubles from min_ms to max_ms
```

**Acceptance:** All three implementations compile and behave correctly.

---

### Task 1.4 — Module Exports (`src/policies/mod.rs`)

```rust
pub mod logging;
pub mod retry;

pub use logging::{LoggingPolicy, NoLogging, ConsoleLogger, FileLogger};
pub use retry::{RetryPolicy, NoRetry, SpinRetry, ExponentialBackoff};
```

---

### Task 1.5 — Main Stub (`src/main.rs`)

```rust
mod tasks;
mod policies;
mod backend;
mod ui;
mod state;

fn main() {
    println!("Alex CRM starting...");
    // TODO: Initialize in Phase 4
}
```

---

## Phase 2 — Backend / Database Layer (Rust)

### Task 2.1 — Database Schema (`src/backend/schema.rs`)

**Goal:** Idempotent schema bootstrap in a single function called at startup.

```rust
pub fn bootstrap_schema(conn: &Connection) -> Result<()> {
    // CREATE TABLE IF NOT EXISTS companies...
    // CREATE TABLE IF NOT EXISTS companies_fts...
    // CREATE TRIGGERs for FTS5 sync
}

pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    // PRAGMA foreign_keys = ON;
    // PRAGMA journal_mode = WAL;
    // etc.
}
```

**Acceptance:** Schema function idempotent; tables and triggers created correctly.

---

### Task 2.2 — DB Worker (`src/backend/db_worker.rs`)

**Goal:** Single thread that owns the SQLite connection and processes DbTask requests.

```rust
pub struct DbWorker {
    db_path: String,
}

impl DbWorker {
    pub fn new(db_path: String) -> Self { ... }

    pub fn run(&self, rx: Receiver<DbTask>, ui_tx: Sender<UiTask>) {
        // Main loop: recv DbTask, process, send UiTask back
    }
}
```

**Acceptance:** Worker receives tasks, processes them, sends results back to UI.

---

### Task 2.3 — Backup Worker (`src/backend/backup_worker.rs`)

**Goal:** Second thread for scheduled backups.

```rust
pub struct BackupWorker {
    source_db: String,
    backup_db: String,
    interval: Duration,
}

impl BackupWorker {
    pub fn run(&self, rx: Receiver<()>, ui_tx: Sender<UiTask>) {
        // Wait on interval, then back up source_db → backup_db
    }
}
```

**Acceptance:** Backup runs periodically without blocking main operations.

---

## Phase 3 — Frontend / UI Layer (FLTK-rs)

### Task 3.1 — Main Window (`src/ui/main_window.rs`)

**Goal:** FLTK main window with 4 tabs.

```rust
pub fn build_main_window(
    db_tx: Sender<DbTask>,
    ui_rx: Receiver<UiTask>,
) -> window::Window {
    // Create 900x650 window, centered on screen
    // Add Tabs: Companies, Contacts, Activities, Logs
}
```

---

### Task 3.2 — Companies Tab (`src/ui/companies_tab.rs`)

**Goal:** Scrollable, searchable list of companies.

```rust
pub fn build_companies_tab(
    db_tx: Sender<DbTask>,
    ui_rx: Receiver<UiTask>,
) -> (Group, HoldBrowser) {
    // Search input, company browser, buttons
    // On search change: post FetchCompaniesReq
    // On row double-click: open detail window
}
```

---

### Task 3.3 — Detail Window (`src/ui/detail_window.rs`)

**Goal:** Non-modal window to edit company details.

```rust
pub fn open_detail_window(
    company_id: i64,
    db_tx: Sender<DbTask>,
) {
    // Fields: name, county, contact first/last
    // Save button: post UpdateCompanyReq
    // Side effect: open file explorer to Company/YYYY/MM
}
```

---

### Task 3.4 — New Company Form (`src/ui/new_company_form.rs`)

**Goal:** Dialog to create a new company.

```rust
pub fn open_new_company_form(
    db_tx: Sender<DbTask>,
) {
    // Fields: name (required), county, contact first/last
    // Validation: name non-empty
    // Save: post InsertCompanyReq
}
```

---

## Phase 4 — Main App Wiring (`src/main.rs` + `src/app.rs`)

### Task 4.1 — App Template (`src/app.rs`)

```rust
pub struct App<L: LoggingPolicy, R: RetryPolicy> {
    logger: Arc<L>,
    retry_policy: Arc<R>,
    db_tx: Sender<DbTask>,
    ui_rx: Receiver<UiTask>,
}

impl<L: LoggingPolicy, R: RetryPolicy> App<L, R> {
    pub fn run(&self) {
        // Spawn DB worker thread
        // Spawn backup worker thread
        // Start FLTK event loop
        // Drain ui_rx on each idle tick, update widgets
    }
}
```

---

### Task 4.2 — Main Entry Point (`src/main.rs`)

```rust
fn main() {
    let logger = Arc::new(ConsoleLogger::new(true));
    let retry = Arc::new(ExponentialBackoff::new(1, 64));

    let (db_tx, db_rx) = crossbeam::channel::bounded(256);
    let (ui_tx, ui_rx) = crossbeam::channel::unbounded();

    // Spawn DB worker
    thread::spawn(move || {
        let worker = DbWorker::new("data/notes_app.db");
        worker.run(db_rx, ui_tx);
    });

    // Build and run app
    let app = App { logger, retry_policy, db_tx, ui_rx };
    app.run();
}
```

---

## Phase 5 — Testing

### Task 5.1 — Unit Tests (`src/**/*.rs`)

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_fetch_companies() { ... }

    #[test]
    fn test_insert_company_duplicate() { ... }

    #[test]
    fn test_logging_policies() { ... }
}
```

---

## Phase 6 — Building & Running

### Task 6.1 — Build

```bash
cd alex-crm-rs

# Download dependencies and compile
cargo build --release

# Output: ./target/release/alex-crm (or .exe on Windows)
```

---

## Implementation Order

```
Phase 1: Foundation
  1.1 Message types (tasks.rs)
  1.2 Logging policy trait + 3 implementations
  1.3 Retry policy trait + 3 implementations
  1.4 Module exports (policies/mod.rs)
  1.5 Main stub

Phase 2: Backend
  2.1 Database schema bootstrap
  2.2 DB worker (recv DbTask, process, send UiTask)
  2.3 Backup worker (scheduled backups)
  2.4 State management (shared channels/logger)

Phase 3: Frontend
  3.1 Main window with tabs
  3.2 Companies tab (search + list)
  3.3 Detail window (edit company)
  3.4 New company form
  3.5 Logs tab (daily log editor)

Phase 4: Wiring
  4.1 App<L, R> template
  4.2 Main entry point

Phase 5: Testing
  5.1 Unit tests

Phase 6: Build & Polish
  6.1 Release build
  6.2 Comments review
  6.3 README
```

---

## Open Decisions

| # | Question | Default |
|---|---|---|
| D1 | Logging: console or file in dev? | Console (ConsoleLogger) |
| D2 | DB thread: std::thread or Tokio? | std::thread (simpler) |
| D3 | SPSC capacity (power of 2)? | 256 |
| D4 | Backup interval? | 300 seconds (5 min) |
| D5 | FTS5 tokenizer? | porter (standard) |
