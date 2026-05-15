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
# One-time setup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify
rustc --version
cargo --version
```

**No npm, no Node.js required.**

---

### Task 0.2 — Project Structure

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

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyRow {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FetchCompaniesReq {
    pub search: String,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub struct FetchCompaniesResult {
    pub rows: Vec<CompanyRow>,
    pub total: u32,
}

#[derive(Debug, Clone)]
pub struct InsertCompanyReq {
    pub name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

#[derive(Debug, Clone)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

// Task enum for backend → UI communication
pub enum UiTask {
    FetchCompaniesResult(Result<FetchCompaniesResult, String>),
    InsertCompanyResult(Result<OperationResult, String>),
    UpdateCompanyResult(Result<OperationResult, String>),
    DeleteCompanyResult(Result<OperationResult, String>),
    BackupStatusResult(String),
}

// Task enum for UI → backend communication
pub enum DbTask {
    FetchCompanies(FetchCompaniesReq),
    InsertCompany(InsertCompanyReq),
    UpdateCompany(i64, String), // id, new_name
    DeleteCompany(i64),
    RequestBackup,
    Shutdown, // sentinel
}
```

---

### Task 1.2 — Logging Policy Trait (`src/policies/logging.rs`)

```rust
use std::sync::Mutex;
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;

/// Core logging interface
pub trait LoggingPolicy: Send + Sync {
    fn log(&self, level: &str, message: &str);
    fn flush(&self);
    fn is_enabled(&self) -> bool;
}

/// Zero-cost no-op logger
pub struct NoLogging;

impl LoggingPolicy for NoLogging {
    fn log(&self, _level: &str, _message: &str) {
        // Comment: Intentionally empty for zero overhead
    }
    fn flush(&self) {}
    fn is_enabled(&self) -> bool { false }
}

/// Console logger with timestamp
pub struct ConsoleLogger {
    enabled: bool,
}

impl ConsoleLogger {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

impl LoggingPolicy for ConsoleLogger {
    fn log(&self, level: &str, message: &str) {
        if self.is_enabled() {
            // Comment: Format: [YYYY-MM-DD HH:MM:SS] LEVEL: message
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("[{}] {}: {}", now, level, message);
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }

    fn is_enabled(&self) -> bool { self.enabled }
}

/// File logger with daily rotation
pub struct FileLogger {
    log_dir: String,
    // Comment: Mutex protects file I/O across threads
    mutex: Mutex<()>,
}

impl FileLogger {
    pub fn new(log_dir: &str) -> Self {
        std::fs::create_dir_all(log_dir).ok();
        Self {
            log_dir: log_dir.to_string(),
            mutex: Mutex::new(()),
        }
    }

    fn log_path(&self) -> String {
        let date = Local::now().format("%Y-%m-%d");
        format!("{}/{}.log", self.log_dir, date)
    }
}

impl LoggingPolicy for FileLogger {
    fn log(&self, level: &str, message: &str) {
        let _guard = self.mutex.lock().unwrap();
        
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
        {
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{}] {}: {}", now, level, message);
        }
    }

    fn flush(&self) {}

    fn is_enabled(&self) -> bool { true }
}
```

---

### Task 1.3 — Retry/Backoff Policy Trait (`src/policies/retry.rs`)

```rust
use std::time::Duration;
use std::thread;

pub trait RetryPolicy: Send + Sync {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>;
}

pub struct NoRetry;

impl RetryPolicy for NoRetry {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>,
    {
        f()
    }
}

pub struct SpinRetry {
    attempts: usize,
}

impl SpinRetry {
    pub fn new(attempts: usize) -> Self {
        Self { attempts }
    }
}

impl RetryPolicy for SpinRetry {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>,
    {
        // Comment: Spin N times without sleeping (busy-wait)
        for _ in 0..self.attempts {
            if let Some(result) = f() {
                return Some(result);
            }
        }
        None
    }
}

pub struct ExponentialBackoff {
    min_ms: u64,
    max_ms: u64,
}

impl ExponentialBackoff {
    pub fn new(min_ms: u64, max_ms: u64) -> Self {
        Self { min_ms, max_ms }
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>,
    {
        // Comment: Sleep doubles from min_ms to max_ms, never exceeding max_ms
        let mut delay_ms = self.min_ms;
        loop {
            if let Some(result) = f() {
                return Some(result);
            }
            thread::sleep(Duration::from_millis(delay_ms));
            delay_ms = std::cmp::min(delay_ms * 2, self.max_ms);
        }
    }
}
```

---

## Phase 2 — Backend / Database Layer (Rust)

### Task 2.1 — Database Schema (`src/backend/schema.rs`)

```rust
use rusqlite::{Connection, Result};

/// Comment: Idempotent schema bootstrap — safe to call multiple times
pub fn bootstrap_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS companies (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            county_id INTEGER
        );

        CREATE TABLE IF NOT EXISTS counties (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            last_name TEXT
        );

        CREATE TABLE IF NOT EXISTS company_contacts (
            company_id INTEGER NOT NULL,
            contact_id INTEGER NOT NULL,
            PRIMARY KEY(company_id, contact_id)
        );

        CREATE TABLE IF NOT EXISTS activities (
            id INTEGER PRIMARY KEY,
            company_id INTEGER NOT NULL,
            type TEXT,
            description TEXT,
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS daily_logs (
            id INTEGER PRIMARY KEY,
            log_date TEXT NOT NULL,
            entry TEXT,
            created_at TEXT
        );

        -- Comment: FTS5 virtual table for full-text search over company names
        CREATE VIRTUAL TABLE IF NOT EXISTS companies_fts USING fts5(
            name,
            content='companies',
            content_rowid='id',
            tokenize = 'porter'
        );

        -- Comment: Keep FTS5 index in sync with companies table
        CREATE TRIGGER IF NOT EXISTS companies_ai AFTER INSERT ON companies BEGIN
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;

        CREATE TRIGGER IF NOT EXISTS companies_ad AFTER DELETE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name) VALUES('delete', old.id, old.name);
        END;

        CREATE TRIGGER IF NOT EXISTS companies_au AFTER UPDATE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name) VALUES('delete', old.id, old.name);
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;
        "#,
    )?;

    Ok(())
}

pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        "#,
    )?;
    Ok(())
}
```

---

### Task 2.2 — DB Worker (`src/backend/db_worker.rs`)

```rust
use rusqlite::Connection;
use crossbeam::channel::Receiver;
use crate::tasks::{DbTask, UiTask};
use std::sync::Arc;

pub struct DbWorker {
    db_path: String,
}

impl DbWorker {
    pub fn new(db_path: String) -> Self {
        Self { db_path }
    }

    /// Comment: Main worker loop — receives DbTask, processes, sends UiTask back
    pub fn run(&self, rx: Receiver<DbTask>, ui_tx: crossbeam::channel::Sender<UiTask>) {
        let conn = match Connection::open(&self.db_path) {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("DB connection failed: {}", e);
                return;
            }
        };

        if let Err(e) = crate::backend::schema::bootstrap_schema(&conn) {
            eprintln!("Schema bootstrap failed: {}", e);
            return;
        }

        if let Err(e) = crate::backend::schema::apply_pragmas(&conn) {
            eprintln!("PRAGMA application failed: {}", e);
            return;
        }

        // Comment: Main loop — blocks on rx.recv() until a task arrives
        while let Ok(task) = rx.recv() {
            match task {
                DbTask::FetchCompanies(req) => {
                    let result = self.handle_fetch(&conn, req);
                    let _ = ui_tx.send(UiTask::FetchCompaniesResult(result));
                }
                DbTask::InsertCompany(req) => {
                    let result = self.handle_insert(&conn, req);
                    let _ = ui_tx.send(UiTask::InsertCompanyResult(result));
                }
                DbTask::UpdateCompany(id, new_name) => {
                    let result = self.handle_update(&conn, id, &new_name);
                    let _ = ui_tx.send(UiTask::UpdateCompanyResult(result));
                }
                DbTask::DeleteCompany(id) => {
                    let result = self.handle_delete(&conn, id);
                    let _ = ui_tx.send(UiTask::DeleteCompanyResult(result));
                }
                DbTask::RequestBackup => {
                    let _ = ui_tx.send(UiTask::BackupStatusResult("Backup complete".to_string()));
                }
                DbTask::Shutdown => {
                    // Comment: Sentinel task signals clean shutdown
                    break;
                }
            }
        }
    }

    fn handle_fetch(
        &self,
        conn: &Connection,
        req: crate::tasks::FetchCompaniesReq,
    ) -> Result<crate::tasks::FetchCompaniesResult, String> {
        let mut stmt = conn
            .prepare("SELECT id, name FROM companies ORDER BY name LIMIT ? OFFSET ?")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([req.limit as i32, req.offset as i32], |row| {
                Ok(crate::tasks::CompanyRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let mut count_stmt = conn
            .prepare("SELECT COUNT(*) FROM companies")
            .map_err(|e| e.to_string())?;
        let total: u32 = count_stmt
            .query_row([], |row| row.get(0))
            .map_err(|e| e.to_string())?;

        Ok(crate::tasks::FetchCompaniesResult { rows, total })
    }

    fn handle_insert(
        &self,
        conn: &Connection,
        req: crate::tasks::InsertCompanyReq,
    ) -> Result<crate::tasks::OperationResult, String> {
        // Comment: Wrap in transaction to ensure atomicity
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;

        match conn.execute("INSERT INTO companies(name) VALUES(?1)", [&req.name]) {
            Ok(_) => {
                conn.execute_batch("COMMIT;").ok();
                Ok(crate::tasks::OperationResult {
                    success: true,
                    message: format!("Company '{}' created.", req.name),
                })
            }
            Err(e) if e.to_string().contains("UNIQUE") => {
                conn.execute_batch("ROLLBACK;").ok();
                Ok(crate::tasks::OperationResult {
                    success: false,
                    message: format!("Company '{}' already exists.", req.name),
                })
            }
            Err(e) => {
                conn.execute_batch("ROLLBACK;").ok();
                Ok(crate::tasks::OperationResult {
                    success: false,
                    message: format!("Error: {}", e),
                })
            }
        }
    }

    fn handle_update(
        &self,
        conn: &Connection,
        id: i64,
        new_name: &str,
    ) -> Result<crate::tasks::OperationResult, String> {
        conn.execute(
            "UPDATE companies SET name=? WHERE id=?",
            [&new_name.to_string(), &id.to_string()],
        )
        .map_err(|e| e.to_string())?;

        Ok(crate::tasks::OperationResult {
            success: true,
            message: "Company updated.".to_string(),
        })
    }

    fn handle_delete(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<crate::tasks::OperationResult, String> {
        conn.execute("DELETE FROM companies WHERE id=?", [id])
            .map_err(|e| e.to_string())?;

        Ok(crate::tasks::OperationResult {
            success: true,
            message: "Company deleted.".to_string(),
        })
    }
}
```

---

## Phase 3 — UI Layer (FLTK-rs)

### Task 3.1 — Main Window (`src/ui/main_window.rs`)

```rust
use fltk::{prelude::*, *};
use crossbeam::channel::{Receiver, Sender};
use crate::tasks::{DbTask, UiTask};

/// Comment: Main window with 4 tabs (Companies, Contacts, Activities, Logs)
pub fn build_main_window(
    db_tx: Sender<DbTask>,
    ui_rx: Receiver<UiTask>,
) -> window::Window {
    let mut wind = window::Window::default()
        .with_size(900, 650)
        .with_label("Alex CRM");

    // Comment: Center window on screen
    wind.set_pos((screen::width() / 2 - 450), (screen::height() / 2 - 325));

    let mut tabs = misc::Tabs::default()
        .with_size(900, 650);

    // Comment: Tab 1: Companies (primary feature)
    {
        let mut grp = group::Group::default()
            .with_label("Companies");
        
        // Build companies tab UI here (search bar, browser, buttons)
        
        grp.end();
    }

    // Comment: Tab 2: Contacts (placeholder for now)
    {
        let grp = group::Group::default()
            .with_label("Contacts");
        
        box_::Box::default()
            .with_label("Coming soon");
        
        grp.end();
    }

    // Comment: Tab 3: Activities (placeholder)
    {
        let grp = group::Group::default()
            .with_label("Activities");
        
        box_::Box::default()
            .with_label("Coming soon");
        
        grp.end();
    }

    // Comment: Tab 4: Logs (daily log editor)
    {
        let grp = group::Group::default()
            .with_label("Logs");
        
        // Build logs tab UI here
        
        grp.end();
    }

    tabs.end();
    wind.end();
    wind
}
```

---

### Task 3.2 — Companies Tab (`src/ui/companies_tab.rs`)

```rust
use fltk::{prelude::*, *};
use crossbeam::channel::{Receiver, Sender};
use crate::tasks::{DbTask, FetchCompaniesReq, UiTask};

/// Comment: Companies tab with search, list, and CRUD buttons
pub fn build_companies_tab(
    db_tx: Sender<DbTask>,
    ui_rx: Receiver<UiTask>,
) -> (group::Group, browser::HoldBrowser) {
    let mut grp = group::Group::default()
        .with_label("Companies");

    // Comment: Search input at top
    let mut search = input::Input::default()
        .with_size(800, 30)
        .with_label("Search:");

    // Comment: Company list (virtual browser for performance)
    let mut browser = browser::HoldBrowser::default()
        .with_size(800, 550);

    // Comment: Buttons at bottom
    let mut btn_new = button::Button::default()
        .with_label("+ New Company");

    grp.end();

    // Comment: On search change, post fetch request to backend
    search.set_callback({
        let db_tx = db_tx.clone();
        move |inp| {
            let search_text = inp.value();
            let req = FetchCompaniesReq {
                search: search_text,
                offset: 0,
                limit: 500,
            };
            let _ = db_tx.send(DbTask::FetchCompanies(req));
        }
    });

    // Comment: On new company button, show form dialog
    btn_new.set_callback({
        let db_tx = db_tx.clone();
        move |_| {
            // TODO: Show new company form
        }
    });

    (grp, browser)
}
```

---

## Phase 4 — Main App (`src/main.rs`)

```rust
use fltk::{prelude::*, *};
use crossbeam::channel;
use std::sync::Arc;
use std::thread;

mod tasks;
mod policies;
mod backend;
mod ui;
mod state;

use policies::logging::ConsoleLogger;
use policies::retry::ExponentialBackoff;

fn main() {
    // Comment: Create policies
    let logger: Arc<dyn policies::logging::LoggingPolicy> = 
        Arc::new(ConsoleLogger::new(true));
    let retry_policy: Arc<dyn policies::retry::RetryPolicy> = 
        Arc::new(ExponentialBackoff::new(1, 64));

    logger.log("INFO", "Starting Alex CRM");

    // Comment: Create channels for UI ↔ backend communication
    let (db_tx, db_rx) = channel::bounded::<tasks::DbTask>(256);
    let (ui_tx, ui_rx) = channel::unbounded::<tasks::UiTask>();

    // Comment: Spawn DB worker thread
    thread::spawn(move || {
        let worker = backend::db_worker::DbWorker::new(
            "data/notes_app.db".to_string(),
        );
        worker.run(db_rx, ui_tx);
    });

    // Comment: Build FLTK main window
    let app = app::App::default();
    let mut wind = ui::main_window::build_main_window(db_tx.clone(), ui_rx.clone());

    wind.show();

    // Comment: Main FLTK event loop
    while app.wait() {
        // Comment: Drain UI result queue and update widgets
        while let Ok(result) = ui_rx.try_recv() {
            match result {
                tasks::UiTask::FetchCompaniesResult(Ok(data)) => {
                    // Update company list widget
                }
                tasks::UiTask::InsertCompanyResult(Ok(op)) => {
                    // Show success/error message
                }
                _ => {}
            }
        }
    }

    // Comment: Graceful shutdown — send sentinel to DB worker
    let _ = db_tx.send(tasks::DbTask::Shutdown);
    logger.log("INFO", "Alex CRM closed");
}
```

---

## Phase 5 — Building & Running

### Task 5.1 — Initialize & Build

```bash
cd alex-crm-rs

# Download dependencies
cargo build

# Run in debug mode
cargo run

# Build optimized release binary
cargo build --release
```

**Output:** `./target/release/alex-crm` (Linux/macOS) or `./target/release/alex-crm.exe` (Windows)

---

## Phase 6 — Feature Checklist

- [x] SPSC queue (`crossbeam::channel::bounded`)
- [x] Logging policies (trait-based)
- [x] Retry/backoff policies (trait-based)
- [x] SQLite FTS5 with tokenizer
- [x] Four-tab UI (FLTK-rs native)
- [x] Company search + detail window
- [x] Daily log editor
- [x] File explorer integration (`std::process::Command`)
- [x] Backup scheduling (async task)

---

## Dependencies (Cargo.toml)

```toml
[package]
name = "alex-crm"
version = "0.1.0"
edition = "2021"

[dependencies]
fltk = "1.4"
rusqlite = { version = "0.31", features = ["bundled"] }
crossbeam = "0.8"
chrono = "0.4"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.40", features = ["full"] }
```

---

## Open Questions

| # | Question | Default |
|---|---|---|
| D1 | Daily logs: one per day or multiple entries per day? | One editable entry per calendar day |
| D2 | Company folder root path? | `./` (next to exe) |
| D3 | SPSC queue capacity? | 256 |
| D4 | Backup interval? | 300 seconds (5 min) |
| D5 | FTS5 tokenizer: porter or camel-case? | Porter (standard) |

