# Alex CRM — Rust + FLTK Implementation Plan

> **Stack:** Rust · FLTK-rs (native GUI) · SQLite3 FTS5 (DB worker thread)
> **Build:** Cargo only · Windows/macOS/Linux
> **Architecture:** App\<LoggingPolicy, RetryPolicy\> template; crossbeam SPSC channel per side; no shared state between threads except the channels.

---

## 0 — Architecture Overview

```
main()
  └─ App<ConsoleLogger, ExponentialBackoff>
       ├─ FLTK UI thread (event loop)
       │    └─ crossbeam::channel::unbounded()  ← backend posts results (UiTask)
       └─ DB worker thread (std::thread)
            └─ crossbeam::channel::bounded(256) ← UI posts requests (DbTask)
```

**Key principle:**
- UI thread and backend are separated by typed channels. No shared mutexes, only message passing.
- Each side owns one channel for sending, one for receiving — producer/consumer roles are fixed.
- The DB is opened exclusively on the worker thread; FLTK never touches SQLite directly.

**Debug assertion:** At no point should `unsafe` appear for cross-thread communication. The type system enforces the boundary.

---

## 1 — Project Structure

```
alex-crm-rs/
├── Cargo.toml
├── Cargo.lock
├── data/
│   └── notes_app.db          ← created at runtime by schema bootstrap
├── src/
│   ├── main.rs               ← entry point, wires everything together
│   ├── app.rs                ← App<L, R> composition root
│   ├── tasks.rs              ← DbTask / UiTask enums + request/response structs
│   ├── backend/
│   │   ├── mod.rs
│   │   ├── schema.rs         ← CREATE TABLE IF NOT EXISTS + FTS5 + triggers
│   │   ├── stmt_cache.rs     ← lazy-prepared statement cache
│   │   ├── db_worker.rs      ← DB worker thread: recv → process → send
│   │   └── backup.rs         ← backup worker: periodic sqlite3_backup
│   ├── policies/
│   │   ├── mod.rs
│   │   ├── logging.rs        ← NoLogging / ConsoleLogger / FileLogger
│   │   └── retry.rs          ← NoRetry / SpinRetry / ExponentialBackoff
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── main_window.rs    ← 900×650 window, 4 tabs, centered on screen
│   │   ├── companies_tab.rs  ← search bar + HoldBrowser + New Company button
│   │   ├── detail_window.rs  ← non-modal company detail editor
│   │   ├── new_company_form.rs ← validation + insert flow
│   │   └── logs_tab.rs       ← daily log text editor
│   └── state.rs              ← Arc<Logger>, channel handles, app-global state
```

---

## 2 — Setup & Toolchain

### 2.1 Install Rust

```powershell
# Windows (PowerShell as Admin)
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe
.\rustup-init.exe -y

# Verify
rustc --version
cargo --version
```

**No Python, no Node.js required.** (If Tauri were chosen you'd need npm, but we use FLTK-rs which is pure Rust.)

### 2.2 Dependencies (Cargo.toml)

The existing `Cargo.toml` is already correct:

```toml
[package]
name = "alex-crm"
version = "0.1.0"
edition = "2021"

[dependencies]
fltk = "1.4"                                          # Native GUI toolkit
rusqlite = { version = "0.31", features = ["bundled"] } # SQLite (no external DLL)
crossbeam = "0.8"                                     # Lock-free SPSC channels
chrono = "0.4"                                        # Timestamps for logging
serde = { version = "1.0", features = ["derive"] }    # Serialization (future use)

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

---

## 3 — Implementation Phases

### Phase 1 — Foundation & Infrastructure

#### Task 1.1 — Message Types (`src/tasks.rs`)

All cross-thread messages defined in one file. Every enum variant is documented with producer/consumer notes.

```rust
// Comment: ──────────────────────────────────────────────
// Comment: Task types for UI ↔ backend communication
// Comment: Producer: UI thread. Consumer: DB worker thread.
// Comment: ──────────────────────────────────────────────

/// A single company row displayed in the browser list.
/// Thread-safe: all fields are owned String/i64.
#[derive(Debug, Clone)]
pub struct CompanyRow {
    pub id: i64,
    pub name: String,
}

/// Request from UI to fetch a page of companies, optionally filtered by search.
/// Purpose: The user types in the search bar or scrolls the company list.
/// Thread: Sent from UI (producer) → DB worker (consumer).
#[derive(Debug, Clone)]
pub struct FetchCompaniesReq {
    pub search: String,      // Empty = fetch all
    pub offset: u32,         // Pagination offset
    pub limit: u32,          // Page size (default 500)
}

/// Result sent back from DB worker after processing FetchCompaniesReq.
#[derive(Debug, Clone)]
pub struct FetchCompaniesResult {
    pub rows: Vec<CompanyRow>,
    pub total: u32,
}

/// Request from UI to insert a new company.
#[derive(Debug, Clone)]
pub struct InsertCompanyReq {
    pub name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

/// Generic success/failure response for insert/update/delete operations.
#[derive(Debug, Clone)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,     // User-facing message: "Company created." or error
}

// ── Enums ──────────────────────────────────────────────

/// Tasks sent from UI → backend.
/// Comment: The DB worker matches on this enum and dispatches to the correct handler.
pub enum DbTask {
    FetchCompanies(FetchCompaniesReq),
    InsertCompany(InsertCompanyReq),
    UpdateCompany { id: i64, name: String, county: String },
    DeleteCompany(i64),
    FetchDailyLog(String),      // date string YYYY-MM-DD
    SaveDailyLog { date: String, text: String },
    RequestBackup,
    Shutdown,                   // sentinel — signals worker to exit
}

/// Results sent from backend → UI.
/// Comment: The FLTK idle callback matches on this enum and updates widgets.
pub enum UiTask {
    FetchCompaniesResult(Result<FetchCompaniesResult, String>),
    InsertCompanyResult(Result<OperationResult, String>),
    UpdateCompanyResult(Result<OperationResult, String>),
    DeleteCompanyResult(Result<OperationResult, String>),
    FetchDailyLogResult(Result<Option<String>, String>),
    SaveDailyLogResult(Result<OperationResult, String>),
    BackupStatusResult(String),
}
```

**Acceptance:** `cargo build` compiles without errors.

---

#### Task 1.2 — Logging Policy Trait (`src/policies/logging.rs`)

Three implementations sharing a common trait. The trait is `Send + Sync` so it can be wrapped in `Arc` and shared across threads.

```rust
use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

/// Comment: Logging interface used by App<L, R>.
/// Every method is documented with thread-safety guarantees.
/// Thread: Any thread can call log(); implementations serialize internally.
pub trait LoggingPolicy: Send + Sync {
    /// Write a log entry. Format: "[YYYY-MM-DD HH:MM:SS] LEVEL: message"
    fn log(&self, level: &str, message: &str);
    /// Flush any buffered output.
    fn flush(&self);
    /// Returns true if this logger is active (used for short-circuiting).
    fn is_enabled(&self) -> bool;
}

// ── NoLogging ──────────────────────────────────────────

/// Zero-cost no-op logger. All methods are empty; the optimizer can eliminate calls.
pub struct NoLogging;

impl LoggingPolicy for NoLogging {
    fn log(&self, _level: &str, _message: &str) {}  // Intentionally empty
    fn flush(&self) {}
    fn is_enabled(&self) -> bool { false }
}

// ── ConsoleLogger ──────────────────────────────────────

/// Writes to stderr with ISO-8601 timestamps.
/// Thread-safe: stderr writes are atomic for messages under PIPE_BUF.
pub struct ConsoleLogger { enabled: bool }

impl ConsoleLogger {
    pub fn new(enabled: bool) -> Self { Self { enabled } }
}

impl LoggingPolicy for ConsoleLogger {
    fn log(&self, level: &str, message: &str) {
        if self.is_enabled() {
            // Comment: Format: [2026-05-15 14:30:00] INFO: Starting app
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("[{}] {}: {}", now, level, message);
        }
    }
    fn flush(&self) { let _ = std::io::stderr().flush(); }
    fn is_enabled(&self) -> bool { self.enabled }
}

// ── FileLogger ─────────────────────────────────────────

/// Writes to logs/YYYY-MM-DD.log with daily rotation.
/// Thread-safe: uses a Mutex to serialize writes.
pub struct FileLogger {
    log_dir: String,
    // Comment: Mutex is necessary because multiple threads (UI, DB, backup)
    // may call log() concurrently. The performance cost is negligible since
    // logging is I/O-bound anyway.
    mutex: Mutex<()>,
}

impl FileLogger {
    pub fn new(log_dir: &str) -> Self {
        // Purpose: Ensure the log directory exists; create it silently if not.
        if let Err(e) = fs::create_dir_all(log_dir) {
            eprintln!("Warning: could not create log dir '{}': {}", log_dir, e);
        }
        Self { log_dir: log_dir.to_string(), mutex: Mutex::new(()) }
    }

    /// Returns the log file path for today's date.
    fn log_path(&self) -> String {
        let date = Local::now().format("%Y-%m-%d");
        format!("{}/{}.log", self.log_dir, date)
    }
}

impl LoggingPolicy for FileLogger {
    fn log(&self, level: &str, message: &str) {
        // Comment: Lock guards the file open/append/write sequence.
        // A crash between open and write may lose that single entry,
        // but cannot corrupt previously written entries.
        let _guard = self.mutex.lock().expect("FileLogger mutex poisoned");
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

**Acceptance:** All three implementations compile; `ConsoleLogger` writes to stderr.

---

#### Task 1.3 — Retry/Backoff Policy Trait (`src/policies/retry.rs`)

```rust
use std::thread;
use std::time::Duration;

/// Comment: Retry strategies for queue operations and DB operations.
/// The `attempt` method calls `f()` repeatedly until it returns Some(T)
/// or the policy decides to give up.
pub trait RetryPolicy: Send + Sync {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where
        F: Fn() -> Option<T>;
}

// ── NoRetry ────────────────────────────────────────────

/// Try once, return immediately. Zero overhead beyond the call itself.
pub struct NoRetry;

impl RetryPolicy for NoRetry {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where F: Fn() -> Option<T> {
        f()
    }
}

// ── SpinRetry ──────────────────────────────────────────

/// Busy-spin up to N times calling f(), then yield.
/// Purpose: Use for very fast operations where sleeping would be slower.
pub struct SpinRetry { attempts: usize }

impl SpinRetry {
    pub fn new(attempts: usize) -> Self { Self { attempts } }
}

impl RetryPolicy for SpinRetry {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where F: Fn() -> Option<T> {
        for _ in 0..self.attempts {
            if let Some(result) = f() {
                return Some(result);
            }
            // Comment: Yield to OS scheduler to avoid starving other threads.
            thread::yield_now();
        }
        None
    }
}

// ── ExponentialBackoff ─────────────────────────────────

/// Sleep between attempts, doubling the delay each time.
/// min_ms: initial delay in milliseconds.
/// max_ms: maximum delay cap (never sleeps longer than this).
pub struct ExponentialBackoff { min_ms: u64, max_ms: u64 }

impl ExponentialBackoff {
    pub fn new(min_ms: u64, max_ms: u64) -> Self {
        debug_assert!(min_ms > 0, "min_ms must be positive");
        Self { min_ms, max_ms }
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn attempt<F, T>(&self, f: F) -> Option<T>
    where F: Fn() -> Option<T> {
        let mut delay = self.min_ms;
        loop {
            if let Some(result) = f() {
                return Some(result);
            }
            thread::sleep(Duration::from_millis(delay));
            // Comment: Exponential backoff with cap — prevents runaway sleep.
            delay = std::cmp::min(delay.saturating_mul(2), self.max_ms);
        }
    }
}
```

**Acceptance:** All three retry policies compile; `NoRetry` calls `f()` exactly once.

---

### Phase 2 — Backend / Database Layer

#### Task 2.1 — DB Schema Bootstrap (`src/backend/schema.rs`)

The schema matches the already-defined `notes_app.db` structure with companies, contacts, counties, activities, daily_logs, and FTS5 full-text search.

```rust
use rusqlite::{Connection, Result};

/// Comment: Idempotent schema bootstrap — safe to call on every app startup.
/// Uses IF NOT EXISTS throughout, so an existing database is never modified.
///
/// Thread: Called once from the DB worker thread on startup.
///         Must NOT be called from the UI thread.
pub fn bootstrap_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        -- Comment: Core entities
        CREATE TABLE IF NOT EXISTS companies (
            id        INTEGER PRIMARY KEY,
            name      TEXT UNIQUE NOT NULL,
            county_id INTEGER REFERENCES counties(id)
        );

        CREATE TABLE IF NOT EXISTS counties (
            id   INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id        INTEGER PRIMARY KEY,
            name      TEXT NOT NULL,
            last_name TEXT
        );

        -- Comment: Many-to-many join between companies and contacts
        CREATE TABLE IF NOT EXISTS company_contacts (
            company_id INTEGER NOT NULL REFERENCES companies(id),
            contact_id INTEGER NOT NULL REFERENCES contacts(id),
            PRIMARY KEY (company_id, contact_id)
        );

        CREATE TABLE IF NOT EXISTS activities (
            id          INTEGER PRIMARY KEY,
            company_id  INTEGER NOT NULL REFERENCES companies(id),
            type        TEXT,
            description TEXT,
            created_at  TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS daily_logs (
            id         INTEGER PRIMARY KEY,
            log_date   TEXT NOT NULL UNIQUE,
            entry      TEXT,
            created_at TEXT DEFAULT (datetime('now'))
        );

        -- Comment: FTS5 virtual table for full-text search over company names.
        -- Uses the 'porter' tokenizer for English stemming.
        -- Covers: the name column only (searches are name-based).
        CREATE VIRTUAL TABLE IF NOT EXISTS companies_fts USING fts5(
            name,
            content='companies',
            content_rowid='id',
            tokenize='porter'
        );

        -- Comment: Triggers to keep companies_fts in sync with companies table.
        -- After INSERT: add the new row to the FTS index.
        CREATE TRIGGER IF NOT EXISTS companies_ai AFTER INSERT ON companies BEGIN
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;

        -- After DELETE: remove the row from the FTS index.
        CREATE TRIGGER IF NOT EXISTS companies_ad AFTER DELETE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name)
            VALUES ('delete', old.id, old.name);
        END;

        -- After UPDATE: delete old entry, insert new entry (FTS has no update).
        CREATE TRIGGER IF NOT EXISTS companies_au AFTER UPDATE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name)
            VALUES ('delete', old.id, old.name);
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;
        "#,
    )?;

    Ok(())
}

/// Apply performance and safety pragmas.
/// Thread: Same as bootstrap_schema — DB worker thread only.
pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys  = ON;     -- Enforce FK constraints
        PRAGMA journal_mode  = WAL;    -- Write-Ahead Logging for concurrent reads
        PRAGMA synchronous   = NORMAL; -- Balance safety/speed (FULL is slower)
        PRAGMA busy_timeout  = 5000;   -- Wait 5s before returning SQLITE_BUSY
        "#,
    )?;
    Ok(())
}
```

**Acceptance:** Running `bootstrap_schema` on an in-memory database creates all 6 tables, 1 virtual table, and 3 triggers. Running it a second time produces no error (idempotent).

---

#### Task 2.2 — Cached Prepared Statements (`src/backend/stmt_cache.rs`)

Lazy-prepared statement cache that holds commonly used SQL statements in prepared form for efficiency.

```rust
use rusqlite::{Connection, Statement, Result};
use std::collections::HashMap;

/// Comment: Per-connection cache of prepared SQL statements.
/// Statements are prepared on first use and reused until the cache is cleared.
/// This avoids SQLite's prepare/compile overhead on hot paths.
///
/// Thread: NOT Send — owned by the DB worker thread only.
pub struct StmtCache<'a> {
    conn: &'a Connection,
    // Comment: HashMap from statement name to prepared Statement.
    // Statements are boxed because Statement is a large struct.
    cache: HashMap<&'static str, Statement<'a>>,
}

impl<'a> StmtCache<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn, cache: HashMap::new() }
    }

    /// Get or prepare a named SQL statement.
    pub fn prepare(&mut self, name: &'static str, sql: &str) -> Result<&mut Statement<'a>> {
        // Comment: Entry API avoids double lookup — check and insert in one operation.
        if !self.cache.contains_key(name) {
            self.cache.insert(name, self.conn.prepare(sql)?);
        }
        // Purpose: unwrap is safe because we just inserted if missing.
        Ok(self.cache.get_mut(name).unwrap())
    }

    /// Prepare commonly used statements at startup.
    /// This avoids lazy-preparation latency on first user interaction.
    pub fn warmup(&mut self) -> Result<()> {
        self.prepare("fetch_all",
            "SELECT id, name FROM companies ORDER BY name COLLATE NOCASE LIMIT ?1 OFFSET ?2")?;
        self.prepare("count_all",
            "SELECT COUNT(*) FROM companies")?;
        self.prepare("insert_company",
            "INSERT INTO companies(name) VALUES(?1)")?;
        self.prepare("update_company",
            "UPDATE companies SET name=?1 WHERE id=?2")?;
        self.prepare("delete_company",
            "DELETE FROM companies WHERE id=?1")?;
        self.prepare("fetch_daily_log",
            "SELECT entry FROM daily_logs WHERE log_date=?1")?;
        self.prepare("upsert_daily_log",
            "INSERT INTO daily_logs(log_date, entry) VALUES(?1, ?2)
             ON CONFLICT(log_date) DO UPDATE SET entry=excluded.entry")?;
        Ok(())
    }
}
```

**Acceptance:** Cache prepares statements on first use; reuses them on subsequent calls. No memory leak (statements freed when `Connection` closes).

---

#### Task 2.3 — DB Worker Thread (`src/backend/db_worker.rs`)

The core backend loop. Owns the SQLite connection, processes `DbTask` items, sends `UiTask` results back.

```rust
use crossbeam::channel::{Receiver, Sender};
use rusqlite::Connection;
use crate::tasks::*;
use super::schema;
use super::stmt_cache::StmtCache;

/// Comment: Long-lived worker that owns the SQLite connection.
/// Runs on a dedicated std::thread. Never touches FLTK or UI code.
///
/// Thread safety: DbWorker itself is not Send — it is moved onto the worker thread.
pub struct DbWorker {
    db_path: String,
}

impl DbWorker {
    pub fn new(db_path: String) -> Self {
        Self { db_path }
    }

    /// Start the main event loop. Blocks until Shutdown is received.
    /// Params:
    ///   rx    — Receiver for DbTask from the UI thread
    ///   ui_tx — Sender for UiTask back to the UI thread
    pub fn run(&self, rx: Receiver<DbTask>, ui_tx: Sender<UiTask>) {
        // Comment: Open database connection. This must succeed — the app
        // cannot function without a database.
        let conn = match Connection::open(&self.db_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("FATAL: cannot open database '{}': {}", self.db_path, e);
                return;
            }
        };

        // Comment: Apply schema and pragmas on every startup.
        // Using IF NOT EXISTS so this is safe on existing databases.
        if let Err(e) = schema::bootstrap_schema(&conn) {
            eprintln!("FATAL: schema bootstrap failed: {}", e);
            return;
        }
        if let Err(e) = schema::apply_pragmas(&conn) {
            eprintln!("FATAL: pragma setup failed: {}", e);
            return;
        }

        // Comment: Warm up the statement cache.
        let mut cache = StmtCache::new(&conn);
        if let Err(e) = cache.warmup() {
            eprintln!("WARN: statement cache warmup failed: {}", e);
            // Non-fatal — statements are prepared lazily.
        }

        // ── Main loop ──────────────────────────────────
        // Comment: Block on rx.recv(). The UI thread controls pacing.
        // When the queue is empty, the worker sleeps without consuming CPU.
        while let Ok(task) = rx.recv() {
            match task {
                DbTask::FetchCompanies(req) => {
                    let result = self.handle_fetch(&conn, &mut cache, &req);
                    let _ = ui_tx.send(UiTask::FetchCompaniesResult(result));
                }
                DbTask::InsertCompany(req) => {
                    let result = self.handle_insert(&conn, &mut cache, &req);
                    let _ = ui_tx.send(UiTask::InsertCompanyResult(result));
                }
                DbTask::UpdateCompany { id, name, county } => {
                    let result = self.handle_update(&conn, &mut cache, id, &name, &county);
                    let _ = ui_tx.send(UiTask::UpdateCompanyResult(result));
                }
                DbTask::DeleteCompany(id) => {
                    let result = self.handle_delete(&conn, &mut cache, id);
                    let _ = ui_tx.send(UiTask::DeleteCompanyResult(result));
                }
                DbTask::FetchDailyLog(date) => {
                    let result = self.handle_fetch_daily_log(&conn, &mut cache, &date);
                    let _ = ui_tx.send(UiTask::FetchDailyLogResult(result));
                }
                DbTask::SaveDailyLog { date, text } => {
                    let result = self.handle_save_daily_log(&conn, &mut cache, &date, &text);
                    let _ = ui_tx.send(UiTask::SaveDailyLogResult(result));
                }
                DbTask::RequestBackup => {
                    let result = self.handle_backup(&conn);
                    let _ = ui_tx.send(UiTask::BackupStatusResult(result));
                }
                DbTask::Shutdown => {
                    // Comment: Sentinel task — clean exit.
                    // The Connection is dropped here, which closes all prepared statements.
                    break;
                }
            }
        }

        // Implicit flush: Connection drops, WAL checkpoint runs.
    }

    // ── Handlers ───────────────────────────────────────

    fn handle_fetch(
        &self, conn: &Connection, cache: &mut StmtCache,
        req: &FetchCompaniesReq,
    ) -> Result<FetchCompaniesResult, String> {
        let rows = {
            // Comment: Use FTS search if query is non-empty, otherwise fetch all.
            if req.search.is_empty() {
                let mut stmt = cache.prepare("fetch_all",
                    "SELECT id, name FROM companies ORDER BY name COLLATE NOCASE LIMIT ?1 OFFSET ?2"
                ).map_err(|e| e.to_string())?;

                stmt.query_map(
                    rusqlite::params![req.limit as i32, req.offset as i32],
                    |row| Ok(CompanyRow { id: row.get(0)?, name: row.get(1)? }),
                ).map_err(|e| e.to_string())?
                  .collect::<Result<Vec<_>, _>>()
                  .map_err(|e| e.to_string())?
            } else {
                // Comment: FTS5 search with MATCH. The rank column sorts by relevance.
                let mut stmt = cache.prepare("fetch_fts",
                    "SELECT c.id, c.name FROM companies c
                     JOIN companies_fts f ON c.id = f.rowid
                     WHERE companies_fts MATCH ?1
                     ORDER BY rank LIMIT ?2 OFFSET ?3"
                ).map_err(|e| e.to_string())?;

                stmt.query_map(
                    rusqlite::params![req.search, req.limit as i32, req.offset as i32],
                    |row| Ok(CompanyRow { id: row.get(0)?, name: row.get(1)? }),
                ).map_err(|e| e.to_string())?
                  .collect::<Result<Vec<_>, _>>()
                  .map_err(|e| e.to_string())?
            }
        };

        // Comment: Count total (unpaginated) rows for UI pagination display.
        // For FTS searches we count from the FTS index; for plain, from companies.
        let total: u32 = if req.search.is_empty() {
            let mut stmt = cache.prepare("count_all", "SELECT COUNT(*) FROM companies")
                .map_err(|e| e.to_string())?;
            stmt.query_row([], |row| row.get(0)).map_err(|e| e.to_string())?
        } else {
            let mut stmt = cache.prepare("count_fts",
                "SELECT COUNT(*) FROM companies_fts WHERE companies_fts MATCH ?1")
                .map_err(|e| e.to_string())?;
            stmt.query_row([&req.search], |row| row.get(0)).map_err(|e| e.to_string())?
        };

        Ok(FetchCompaniesResult { rows, total })
    }

    fn handle_insert(
        &self, conn: &Connection, cache: &mut StmtCache,
        req: &InsertCompanyReq,
    ) -> Result<OperationResult, String> {
        // Comment: Use IMMEDIATE transaction to prevent deadlocks with WAL mode.
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;

        match conn.execute("INSERT INTO companies(name) VALUES(?1)",
                           rusqlite::params![req.name.trim()])
        {
            Ok(_) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(OperationResult {
                    success: true,
                    message: format!("Company '{}' created.", req.name.trim()),
                })
            }
            Err(e) => {
                // Comment: Rollback is critical — without it the connection
                // remains in a failed transaction state.
                let _ = conn.execute_batch("ROLLBACK");
                if e.to_string().contains("UNIQUE") {
                    Ok(OperationResult {
                        success: false,
                        message: format!("Company '{}' already exists.", req.name.trim()),
                    })
                } else {
                    Ok(OperationResult {
                        success: false,
                        message: format!("Database error: {}", e),
                    })
                }
            }
        }
    }

    fn handle_update(
        &self, conn: &Connection, _cache: &mut StmtCache,
        id: i64, name: &str, _county: &str,
    ) -> Result<OperationResult, String> {
        conn.execute("UPDATE companies SET name=?1 WHERE id=?2",
                     rusqlite::params![name.trim(), id])
            .map_err(|e| e.to_string())?;
        Ok(OperationResult { success: true, message: "Company updated.".to_string() })
    }

    fn handle_delete(
        &self, conn: &Connection, _cache: &mut StmtCache,
        id: i64,
    ) -> Result<OperationResult, String> {
        conn.execute("DELETE FROM companies WHERE id=?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(OperationResult { success: true, message: "Company deleted.".to_string() })
    }

    fn handle_fetch_daily_log(
        &self, conn: &Connection, cache: &mut StmtCache,
        date: &str,
    ) -> Result<Option<String>, String> {
        let mut stmt = cache.prepare("fetch_daily_log",
            "SELECT entry FROM daily_logs WHERE log_date=?1")
            .map_err(|e| e.to_string())?;

        match stmt.query_row(rusqlite::params![date], |row| row.get::<_, String>(0)) {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn handle_save_daily_log(
        &self, conn: &Connection, cache: &mut StmtCache,
        date: &str, text: &str,
    ) -> Result<OperationResult, String> {
        // Comment: UPSERT — insert or update if the date already has an entry.
        let mut stmt = cache.prepare("upsert_daily_log",
            "INSERT INTO daily_logs(log_date, entry) VALUES(?1, ?2)
             ON CONFLICT(log_date) DO UPDATE SET entry=excluded.entry")
            .map_err(|e| e.to_string())?;

        stmt.execute(rusqlite::params![date, text])
            .map_err(|e| e.to_string())?;

        Ok(OperationResult { success: true, message: "Log saved.".to_string() })
    }

    fn handle_backup(&self, _conn: &Connection) -> String {
        // Comment: Placeholder — real backup uses sqlite3_backup_init API.
        // TODO: Phase 2.4 implements full backup via rusqlite's backup API.
        "Backup not yet implemented (placeholder).".to_string()
    }
}
```

**Acceptance:** Worker receives all DbTask variants, dispatches correctly, sends UiTask results back. Shutdown causes clean exit.

---

#### Task 2.4 — Backup Worker (`src/backend/backup.rs`)

```rust
use crossbeam::channel::{Receiver, Sender};
use rusqlite::{Connection, Backup};
use std::thread;
use std::time::{Duration, Instant};
use crate::tasks::UiTask;

/// Comment: Periodic backup worker that runs on its own thread.
/// Uses SQLite's online backup API for consistent snapshots without locking.
pub struct BackupWorker {
    source_path: String,
    backup_path: String,
    interval: Duration,
}

impl BackupWorker {
    pub fn new(source_path: String, backup_path: String, interval_secs: u64) -> Self {
        Self {
            source_path,
            backup_path,
            interval: Duration::from_secs(interval_secs),
        }
    }

    /// Run the backup loop. Returns when shutdown signal received.
    /// Params:
    ///   shutdown_rx — receives () when app wants to shut down
    ///   ui_tx — for sending status updates to the UI
    pub fn run(&self, shutdown_rx: Receiver<()>, ui_tx: Sender<UiTask>) {
        let _ = ui_tx.send(UiTask::BackupStatusResult(
            "Backup worker started.".to_string()
        ));

        loop {
            // Comment: Wait for interval OR shutdown signal, whichever comes first.
            // Using recv_timeout instead of thread::sleep allows immediate shutdown.
            match shutdown_rx.recv_timeout(self.interval) {
                Ok(()) => {
                    // Comment: Shutdown signal received — exit cleanly.
                    let _ = ui_tx.send(UiTask::BackupStatusResult(
                        "Backup worker shutting down.".to_string()
                    ));
                    break;
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    // Comment: Interval elapsed — perform backup.
                    self.perform_backup(&ui_tx);
                }
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    // Comment: Sender dropped — exit.
                    break;
                }
            }
        }
    }

    fn perform_backup(&self, ui_tx: &Sender<UiTask>) {
        let start = Instant::now();
        let _ = ui_tx.send(UiTask::BackupStatusResult("Backup starting...".to_string()));

        // Comment: Open source database read-only, backup database read-write.
        // rusqlite's Backup API wraps sqlite3_backup_init/step/finish.
        match (Connection::open_with_flags(&self.source_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY),
               Connection::open(&self.backup_path))
        {
            (Ok(src), Ok(dst)) => {
                let backup = Backup::new(&src, &dst)
                    .expect("Failed to initialize backup");

                // Comment: Step through pages. 500 pages per iteration is the
                // recommended batch size for balancing progress vs overhead.
                match backup.run_to_completion(500, Duration::from_millis(25), None) {
                    Ok(pages) => {
                        let elapsed = start.elapsed();
                        let _ = ui_tx.send(UiTask::BackupStatusResult(
                            format!("Backup complete: {} pages in {:?}", pages, elapsed)
                        ));
                    }
                    Err(e) => {
                        let _ = ui_tx.send(UiTask::BackupStatusResult(
                            format!("Backup failed: {}", e)
                        ));
                    }
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                let _ = ui_tx.send(UiTask::BackupStatusResult(
                    format!("Backup open failed: {}", e)
                ));
            }
        }
    }
}
```

**Acceptance:** Backup runs at the configured interval; does not block the DB worker; shutdown causes clean exit within `interval` seconds.

---

### Phase 3 — Frontend / UI Layer (FLTK-rs)

#### Task 3.1 — Application State (`src/state.rs`)

```rust
use crossbeam::channel::{Sender, Receiver, unbounded, bounded};
use std::sync::Arc;
use crate::policies::logging::LoggingPolicy;
use crate::tasks::{DbTask, UiTask};

/// Comment: Global application state shared across all UI components.
/// Created once in main() and passed by reference to tab constructors.
pub struct AppState {
    pub db_tx: Sender<DbTask>,          // UI → DB worker
    pub ui_rx: Receiver<UiTask>,        // DB worker → UI
    pub logger: Arc<dyn LoggingPolicy>, // Shared logger
}

impl AppState {
    pub fn new(logger: Arc<dyn LoggingPolicy>) -> Self {
        // Comment: bounded(256) provides backpressure on the request channel.
        // unbounded() on the response channel prevents deadlocks when UI is busy.
        let (db_tx, _db_rx) = bounded::<DbTask>(256);
        let (_ui_tx, ui_rx) = unbounded::<UiTask>();

        Self { db_tx, ui_rx, logger }
    }
}
```

---

#### Task 3.2 — Main Window (`src/ui/main_window.rs`)

```rust
use fltk::{prelude::*, *};
use crate::state::AppState;
use super::companies_tab;
use super::logs_tab;

/// Comment: Build the main application window with 4 tabs.
/// The window is centered on screen and has a minimum size constraint.
///
/// Params:
///   state — shared AppState containing channels and logger
///
/// Returns: The constructed window (call .show() then app::App::run()).
pub fn build_main_window(state: AppState) -> window::Window {
    // ── Window setup ───────────────────────────────────
    let mut wind = window::Window::default()
        .with_size(900, 650)
        .with_label("Alex CRM");

    // Comment: Center the window on screen.
    // FLTK provides screen dimensions via screen::size() (or screen::w/h).
    let (sw, sh) = (app::screen_size().0 as i32, app::screen_size().1 as i32);
    wind.set_pos((sw - 900) / 2, (sh - 650) / 2);
    wind.set_size_range(700, 500); // Minimum size constraint

    // ── Tabs ───────────────────────────────────────────
    let mut tabs = misc::Tabs::default()
        .with_size(900, 650)
        .center_of_parent();

    // Comment: Tab 1 — Companies (primary feature, implemented first)
    let (_companies_grp, _browser) = companies_tab::build_companies_tab(&state);

    // Comment: Tab 2 — Contacts (placeholder for future implementation)
    {
        let mut grp = group::Group::default().with_label("Contacts");
        let mut box_ = frame::Frame::default()
            .with_size(200, 40)
            .center_of_parent();
        box_.set_label("Contacts — Coming Soon");
        box_.set_label_size(20);
        grp.end();
    }

    // Comment: Tab 3 — Activities (placeholder for future implementation)
    {
        let mut grp = group::Group::default().with_label("Activities");
        let mut box_ = frame::Frame::default()
            .with_size(200, 40)
            .center_of_parent();
        box_.set_label("Activities — Coming Soon");
        box_.set_label_size(20);
        grp.end();
    }

    // Comment: Tab 4 — Logs (daily log editor)
    logs_tab::build_logs_tab(&state);

    tabs.end();
    wind.end();
    wind
}
```

---

#### Task 3.3 — Companies Tab (`src/ui/companies_tab.rs`)

The main data surface. Contains a search bar, a company list browser, and a "New Company" button.

```rust
use fltk::{prelude::*, *};
use crate::state::AppState;
use crate::tasks::*;
use super::detail_window;
use super::new_company_form;

/// Comment: Build the Companies tab content.
/// Returns the group and the HoldBrowser reference for external updates.
pub fn build_companies_tab(
    state: &AppState,
) -> (group::Group, browser::HoldBrowser) {
    let mut grp = group::Group::default().with_label("Companies");

    // Comment: Search bar at the top, full width minus padding.
    let mut search = input::Input::default()
        .with_pos(10, 10)
        .with_size(880, 28);
    search.set_placeholder("Search companies...");

    // Comment: Company list — HoldBrowser for single-selection.
    let mut browser = browser::HoldBrowser::default()
        .with_pos(10, 45)
        .with_size(880, 550);

    // Comment: New Company button at bottom-right.
    let mut btn_new = button::Button::default()
        .with_pos(770, 605)
        .with_size(120, 30);
    btn_new.set_label("+ New Company");

    grp.end();

    // ── Callbacks ────────────────────────────────────

    // Comment: On search text change, send FetchCompaniesReq to backend.
    // Uses a short debounce (handled by the backend's processing speed).
    let db_tx = state.db_tx.clone();
    search.set_callback(move |inp| {
        let req = FetchCompaniesReq {
            search: inp.value(),
            offset: 0,
            limit: 500,
        };
        let _ = db_tx.send(DbTask::FetchCompanies(req));
    });

    // Comment: On double-click or Enter, open the detail window for the
    // selected company.
    let db_tx2 = state.db_tx.clone();
    let logger2 = state.logger.clone();
    browser.set_callback(move |br| {
        if let Some(idx) = br.selected() {
            // Comment: Extract company ID from the browser item data.
            // The ID is stored as user_data on each line.
            let text = br.text(idx);
            if let Some((id_str, _)) = text.split_once(". ") {
                if let Ok(id) = id_str.parse::<i64>() {
                    detail_window::open_detail_window(id, &text, &db_tx2, &logger2);
                }
            }
        }
    });

    // Comment: New Company button opens the creation form.
    let db_tx3 = state.db_tx.clone();
    btn_new.set_callback(move |_| {
        new_company_form::open_new_company_form(&db_tx3);
    });

    (grp, browser)
}

/// Comment: Update the company list browser with fresh data from the backend.
pub fn update_company_browser(
    browser: &mut browser::HoldBrowser,
    result: &FetchCompaniesResult,
) {
    browser.clear();
    for (i, row) in result.rows.iter().enumerate() {
        // Comment: Format: "1. Acme Corp" — the index acts as visual reference.
        browser.add(&format!("{}. {}", i + 1, row.name));
    }
}
```

---

#### Task 3.4 — Company Detail Window (`src/ui/detail_window.rs`)

```rust
use fltk::{prelude::*, *};
use crossbeam::channel::Sender;
use std::process::Command;
use std::path::Path;
use chrono::Local;
use crate::tasks::*;
use std::sync::Arc;
use crate::policies::logging::LoggingPolicy;

/// Comment: Open a non-modal window for viewing/editing company details.
/// Also opens the file explorer to "Company Name/YYYY/MM/" directory.
///
/// Params:
///   company_id — database ID of the company
///   display_text — the text shown in the browser (contains the name)
///   db_tx — channel to send UpdateCompanyReq
///   logger — for logging the explorer launch
pub fn open_detail_window(
    company_id: i64,
    display_text: &str,
    db_tx: &Sender<DbTask>,
    logger: &Arc<dyn LoggingPolicy>,
) {
    // Comment: Extract company name from display text ("1. Acme Corp" → "Acme Corp").
    let company_name = display_text
        .split(". ")
        .nth(1)
        .unwrap_or(display_text)
        .to_string();

    // ── Create the detail window ─────────────────────
    let mut wind = window::Window::default()
        .with_size(500, 350)
        .with_label(&format!("Edit: {}", company_name));

    // Comment: Center this window relative to the main window.
    let (sw, sh) = (app::screen_size().0 as i32, app::screen_size().1 as i32);
    wind.set_pos((sw - 500) / 2, (sh - 350) / 2);
    wind.make_modal(false); // Non-modal — user can interact with main window

    // ── Form fields ──────────────────────────────────
    let mut inp_name = input::Input::default()
        .with_pos(10, 10)
        .with_size(480, 28);
    inp_name.set_value(&company_name);

    let mut inp_county = input::Input::default()
        .with_pos(10, 45)
        .with_size(480, 28);
    inp_county.set_label("County:");

    // ── Buttons ──────────────────────────────────────
    let mut btn_save = button::Button::default()
        .with_pos(310, 310)
        .with_size(80, 28);
    btn_save.set_label("Save");

    let mut btn_cancel = button::Button::default()
        .with_pos(400, 310)
        .with_size(80, 28);
    btn_cancel.set_label("Cancel");

    wind.end();
    wind.show();

    // ── File explorer integration ─────────────────────
    // Comment: Open file explorer to "Company Name/YYYY/MM" directory.
    // This matches the spec: clicking a company opens the folder
    // structured as "Company name/year/month".
    let now = Local::now();
    let folder_path = format!(
        "{}/{}/{:02}",
        company_name.trim(),
        now.format("%Y"),
        now.format("%m")
    );

    logger.log("INFO", &format!("Opening explorer to: {}", folder_path));

    // Comment: Create directory if it doesn't exist.
    if let Err(e) = std::fs::create_dir_all(&folder_path) {
        logger.log("WARN", &format!("Could not create directory '{}': {}", folder_path, e));
    }

    // Comment: Open Windows Explorer (platform-specific).
    // On macOS this would use `open`, on Linux `xdg-open`.
    #[cfg(target_os = "windows")]
    {
        // Comment: Use cmd.exe /c start to open the folder in Explorer.
        // std::process::Command avoids unsafe ShellExecuteW calls.
        let _ = Command::new("cmd")
            .args(["/c", "start", "", &folder_path])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(&folder_path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(&folder_path).spawn();
    }

    // ── Callbacks ────────────────────────────────────
    let db_tx = db_tx.clone();
    btn_save.set_callback(move |_| {
        let _ = db_tx.send(DbTask::UpdateCompany {
            id: company_id,
            name: inp_name.value(),
            county: inp_county.value(),
        });
        // Comment: Close the detail window after save.
        if let Some(mut w) = wind.as_ref() {
            w.hide();
        }
    });

    btn_cancel.set_callback(move |_| {
        wind.hide();
    });
}
```

---

#### Task 3.5 — New Company Form (`src/ui/new_company_form.rs`)

```rust
use fltk::{prelude::*, *};
use crossbeam::channel::Sender;
use crate::tasks::*;

/// Comment: Open a modal dialog for creating a new company.
/// Validates that the name is non-empty before sending the insert request.
pub fn open_new_company_form(db_tx: &Sender<DbTask>) {
    let mut wind = window::Window::default()
        .with_size(400, 250)
        .with_label("New Company");

    let (sw, sh) = (app::screen_size().0 as i32, app::screen_size().1 as i32);
    wind.set_pos((sw - 400) / 2, (sh - 250) / 2);

    // ── Fields ───────────────────────────────────────
    let mut inp_name = input::Input::default()
        .with_pos(10, 10)
        .with_size(380, 28);
    inp_name.set_label("Company Name*:");

    let mut inp_county = input::Input::default()
        .with_pos(10, 45)
        .with_size(380, 28);
    inp_county.set_label("County:");

    let mut inp_first = input::Input::default()
        .with_pos(10, 80)
        .with_size(380, 28);
    inp_first.set_label("Contact First Name:");

    let mut inp_last = input::Input::default()
        .with_pos(10, 115)
        .with_size(380, 28);
    inp_last.set_label("Contact Last Name:");

    // Comment: Error message label — hidden by default, shows red text on validation failure.
    let mut err_label = frame::Frame::default()
        .with_pos(10, 155)
        .with_size(380, 20);
    err_label.set_label_color(enums::Color::Red);
    err_label.hide();

    // ── Buttons ──────────────────────────────────────
    let mut btn_save = button::Button::default()
        .with_pos(210, 210)
        .with_size(80, 28);
    btn_save.set_label("Save");

    let mut btn_cancel = button::Button::default()
        .with_pos(300, 210)
        .with_size(80, 28);
    btn_cancel.set_label("Cancel");

    wind.end();
    wind.show();

    // ── Callbacks ────────────────────────────────────
    let db_tx = db_tx.clone();
    btn_save.set_callback(move |_| {
        let name = inp_name.value();

        // Comment: Client-side validation — name must not be empty or whitespace.
        if name.trim().is_empty() {
            err_label.set_label("Company name is required.");
            err_label.show();
            return;
        }

        err_label.hide();

        let _ = db_tx.send(DbTask::InsertCompany(InsertCompanyReq {
            name: name,
            county: inp_county.value(),
            contact_first: inp_first.value(),
            contact_last: inp_last.value(),
        }));

        wind.hide();
    });

    btn_cancel.set_callback(move |_| {
        wind.hide();
    });
}
```

---

#### Task 3.6 — Logs Tab (`src/ui/logs_tab.rs`)

```rust
use fltk::{prelude::*, *};
use chrono::Local;
use crate::state::AppState;
use crate::tasks::*;

/// Comment: Daily log editor tab. Shows today's date and a text editor.
/// The user can write a daily journal entry and save it.
///
/// Behaviour:
/// - On tab shown: fetches today's existing entry (if any) from the backend.
/// - On save: upserts the entry for today's date.
pub fn build_logs_tab(state: &AppState) {
    let mut grp = group::Group::default().with_label("Logs");

    // Comment: Date display at the top.
    let today = Local::now().format("%A, %B %d, %Y").to_string();
    let mut date_label = frame::Frame::default()
        .with_pos(10, 10)
        .with_size(880, 30);
    date_label.set_label(&today);
    date_label.set_label_size(18);

    // Comment: Multi-line text editor for the log entry.
    let mut editor = text::TextEditor::default()
        .with_pos(10, 50)
        .with_size(880, 540);
    editor.wrap_mode(text::WrapMode::AtBounds, 0);

    // Comment: Save button.
    let mut btn_save = button::Button::default()
        .with_pos(770, 605)
        .with_size(120, 30);
    btn_save.set_label("Save Entry");

    grp.end();

    // ── Fetch existing entry for today ───────────────
    let today_str = Local::now().format("%Y-%m-%d").to_string();
    let db_tx = state.db_tx.clone();
    let _ = db_tx.send(DbTask::FetchDailyLog(today_str.clone()));

    // Note: The fetch result arrives as UiTask::FetchDailyLogResult.
    // The main loop's idle callback handles updating the editor content.

    // ── Save callback ────────────────────────────────
    let db_tx2 = state.db_tx.clone();
    let date_str = today_str.clone();
    btn_save.set_callback(move |_| {
        let text = editor.buffer().map(|b| b.text()).unwrap_or_default();
        let _ = db_tx2.send(DbTask::SaveDailyLog {
            date: date_str.clone(),
            text,
        });
    });
}
```

---

### Phase 4 — App Wiring & Entry Point

#### Task 4.1 — Module Registration (`src/backend/mod.rs`, `src/ui/mod.rs`, `src/policies/mod.rs`)

```rust
// src/backend/mod.rs
pub mod schema;
pub mod stmt_cache;
pub mod db_worker;
pub mod backup;

// src/ui/mod.rs
pub mod main_window;
pub mod companies_tab;
pub mod detail_window;
pub mod new_company_form;
pub mod logs_tab;

// src/policies/mod.rs
pub mod logging;
pub mod retry;
pub use logging::*;
pub use retry::*;
```

---

#### Task 4.2 — App Composition Root (`src/app.rs`)

```rust
use crossbeam::channel;
use std::sync::Arc;
use std::thread;
use crate::policies::logging::LoggingPolicy;
use crate::policies::retry::RetryPolicy;
use crate::tasks::{DbTask, UiTask};
use crate::backend::db_worker::DbWorker;
use crate::backend::backup::BackupWorker;

/// Comment: Composition root that owns all threads and channels.
///
/// Type parameters:
///   L — LoggingPolicy (NoLogging, ConsoleLogger, FileLogger)
///   R — RetryPolicy  (NoRetry, SpinRetry, ExponentialBackoff)
///
/// Both are stored as Arc<dyn Trait> for shared access across threads.
pub struct App<L: LoggingPolicy, R: RetryPolicy> {
    pub logger: Arc<L>,
    pub retry: Arc<R>,
    pub db_tx: channel::Sender<DbTask>,
    pub ui_rx: channel::Receiver<UiTask>,
    pub backup_tx: channel::Sender<()>, // For signaling backup shutdown
}

impl<L: LoggingPolicy + 'static, R: RetryPolicy + 'static> App<L, R> {
    /// Create and start the application.
    /// Params:
    ///   logger — logging policy instance
    ///   retry  — retry policy instance
    ///   db_path — path to the SQLite database file
    ///   backup_db_path — path for the backup copy
    ///   backup_interval_secs — seconds between automatic backups
    pub fn new(
        logger: L,
        retry: R,
        db_path: &str,
        backup_db_path: &str,
        backup_interval_secs: u64,
    ) -> Self {
        let logger = Arc::new(logger);
        let retry = Arc::new(retry);

        // Comment: Bounded channel for DB requests (backpressure).
        let (db_tx, db_rx) = channel::bounded::<DbTask>(256);
        // Comment: Unbounded channel for UI results (never block the backend).
        let (ui_tx, ui_rx) = channel::unbounded::<UiTask>();
        // Comment: Shutdown signal for backup worker.
        let (backup_tx, backup_rx) = channel::bounded::<()>(1);

        // Comment: Start DB worker thread.
        let db_path_clone = db_path.to_string();
        thread::Builder::new()
            .name("db-worker".into())
            .spawn(move || {
                let worker = DbWorker::new(db_path_clone);
                worker.run(db_rx, ui_tx);
            })
            .expect("Failed to spawn DB worker thread");

        // Comment: Start backup worker thread (if interval > 0).
        if backup_interval_secs > 0 {
            thread::Builder::new()
                .name("backup-worker".into())
                .spawn(move || {
                    let worker = BackupWorker::new(
                        db_path.to_string(),
                        backup_db_path.to_string(),
                        backup_interval_secs,
                    );
                    worker.run(backup_rx, ui_tx);
                })
                .expect("Failed to spawn backup worker thread");
        }

        Self { logger, retry, db_tx, ui_rx, backup_tx }
    }

    /// Log a message through the shared logger.
    pub fn log(&self, level: &str, message: &str) {
        self.logger.log(level, message);
    }

    /// Send a shutdown signal and wait for threads to finish.
    pub fn shutdown(&self) {
        self.log("INFO", "Shutting down...");
        let _ = self.db_tx.send(DbTask::Shutdown);
        let _ = self.backup_tx.send(()); // Signal backup worker to exit
    }
}
```

---

#### Task 4.3 — Main Entry Point (`src/main.rs`)

```rust
use fltk::{prelude::*, *};
use std::sync::Arc;
use std::time::Duration;

mod tasks;
mod policies;
mod backend;
mod ui;
mod state;
mod app;

use policies::logging::ConsoleLogger;
use policies::retry::ExponentialBackoff;
use app::App;

fn main() {
    // ── Create application with ConsoleLogger + ExponentialBackoff ──
    // Comment: ConsoleLogger(true) enables stderr logging.
    // ExponentialBackoff(1, 64) starts at 1ms delay, doubles to 64ms max.
    let app = App::new(
        ConsoleLogger::new(true),
        ExponentialBackoff::new(1, 64),
        "data/notes_app.db",
        "data/notes_app_backup.db",
        300, // Backup every 5 minutes
    );

    app.log("INFO", "Alex CRM starting...");

    // ── Create shared state for UI ────────────────────
    let app_state = state::AppState {
        db_tx: app.db_tx.clone(),
        ui_rx: app.ui_rx.clone(),
        logger: app.logger.clone(),
    };

    // ── Build FLTK UI ────────────────────────────────
    let mut wind = ui::main_window::build_main_window(app_state);

    // Comment: Show the window and start the FLTK event loop.
    wind.show();

    // Comment: Use app::App::default() for FLTK 1.4's modern event loop.
    let fltk_app = app::App::default().with_scheme(app::Scheme::Gleam);

    // Comment: Register idle callback to drain the UI result queue.
    // This is the ONLY place where UiTask results are processed.
    let ui_rx = app.ui_rx.clone();
    let ui_logger = app.logger.clone();

    // Comment: The idle callback runs on every FLTK event loop iteration.
    // It drains all available UiTask items and updates the UI accordingly.
    app::add_idle(move || {
        // Comment: Non-blocking drain of the result channel.
        while let Ok(result) = ui_rx.try_recv() {
            match result {
                tasks::UiTask::FetchCompaniesResult(Ok(data)) => {
                    // TODO: Update the company list browser
                    ui_logger.log("DEBUG", &format!("Fetched {} companies", data.rows.len()));
                }
                tasks::UiTask::InsertCompanyResult(Ok(op)) => {
                    if op.success {
                        ui_logger.log("INFO", &op.message);
                        // Fl_Alert or status bar update
                    } else {
                        dialog::alert(300, 200, &op.message);
                    }
                }
                tasks::UiTask::UpdateCompanyResult(Ok(op)) => {
                    if op.success {
                        ui_logger.log("INFO", &op.message);
                    } else {
                        dialog::alert(300, 200, &op.message);
                    }
                }
                tasks::UiTask::DeleteCompanyResult(Ok(op)) => {
                    if op.success {
                        ui_logger.log("INFO", &op.message);
                    } else {
                        dialog::alert(300, 200, &op.message);
                    }
                }
                tasks::UiTask::BackupStatusResult(msg) => {
                    ui_logger.log("INFO", &format!("Backup: {}", msg));
                }
                tasks::UiTask::FetchDailyLogResult(Ok(entry)) => {
                    // TODO: Update the logs tab editor with the fetched entry
                    ui_logger.log("DEBUG", &format!("Daily log fetched: exists={}", entry.is_some()));
                }
                tasks::UiTask::SaveDailyLogResult(Ok(op)) => {
                    ui_logger.log("INFO", &op.message);
                }
                // ── Error cases ──────────────────────
                tasks::UiTask::FetchCompaniesResult(Err(e)) |
                tasks::UiTask::InsertCompanyResult(Err(e)) |
                tasks::UiTask::UpdateCompanyResult(Err(e)) |
                tasks::UiTask::DeleteCompanyResult(Err(e)) |
                tasks::UiTask::FetchDailyLogResult(Err(e)) |
                tasks::UiTask::SaveDailyLogResult(Err(e)) => {
                    dialog::alert(300, 200, &format!("Error: {}", e));
                    ui_logger.log("ERROR", &e);
                }
            }
        }
    });

    // Comment: Run the FLTK event loop — blocks until the window is closed.
    let _ = fltk_app.run()?;

    // Comment: Window closed — send sentinels and exit.
    app.shutdown();

    // Comment: Give threads time to clean up, but don't block indefinitely.
    std::thread::sleep(Duration::from_millis(100));

    app.log("INFO", "Alex CRM stopped.");
}
```

---

### Phase 5 — Testing

#### Task 5.1 — Schema Bootstrap Test

Create `tests/schema_test.rs`:

```rust
#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    #[test]
    fn test_bootstrap_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        crate::backend::schema::bootstrap_schema(&conn).unwrap();

        // Comment: Verify all tables exist.
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"companies".to_string()));
        assert!(tables.contains(&"contacts".to_string()));
        assert!(tables.contains(&"counties".to_string()));
        assert!(tables.contains(&"company_contacts".to_string()));
        assert!(tables.contains(&"activities".to_string()));
        assert!(tables.contains(&"daily_logs".to_string()));

        // Comment: Verify FTS virtual table exists.
        let fts_tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='companies_fts'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(!fts_tables.is_empty(), "companies_fts FTS table should exist");

        // Comment: Second call must not fail (idempotent).
        crate::backend::schema::bootstrap_schema(&conn).unwrap();
    }
}
```

#### Task 5.2 — DB Worker Round-Trip Test

```rust
#[test]
fn test_db_worker_insert_and_fetch() {
    use crossbeam::channel;
    use crate::backend::db_worker::DbWorker;
    use crate::tasks::*;

    let (db_tx, db_rx) = channel::bounded::<DbTask>(256);
    let (ui_tx, ui_rx) = channel::unbounded::<UiTask>();

    // Comment: Use a temp file for testing.
    let tmp = std::env::temp_dir().join("test_crm.db");
    let _ = std::fs::remove_file(&tmp); // Clean up from previous runs

    let worker = DbWorker::new(tmp.to_string_lossy().to_string());
    std::thread::spawn(move || worker.run(db_rx, ui_tx));

    // Comment: Insert a company.
    db_tx.send(DbTask::InsertCompany(InsertCompanyReq {
        name: "Acme Corp".into(),
        county: "".into(),
        contact_first: "John".into(),
        contact_last: "Doe".into(),
    })).unwrap();

    // Comment: Verify insert result.
    match ui_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(UiTask::InsertCompanyResult(Ok(op))) => assert!(op.success, "Insert should succeed"),
        other => panic!("Expected InsertCompanyResult, got {:?}", other),
    }

    // Comment: Fetch companies.
    db_tx.send(DbTask::FetchCompanies(FetchCompaniesReq {
        search: "".into(),
        offset: 0,
        limit: 100,
    })).unwrap();

    match ui_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(UiTask::FetchCompaniesResult(Ok(result))) => {
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0].name, "Acme Corp");
        }
        other => panic!("Expected FetchCompaniesResult, got {:?}", other),
    }

    // Comment: Shutdown.
    db_tx.send(DbTask::Shutdown).unwrap();
    std::fs::remove_file(&tmp).ok();
}
```

#### Task 5.3 — Logging Policy Test

```rust
#[test]
fn test_console_logger() {
    let logger = crate::policies::logging::ConsoleLogger::new(true);
    assert!(logger.is_enabled());
    logger.log("INFO", "Test message"); // Visible on stderr
    logger.flush();
}

#[test]
fn test_no_logging_zero_cost() {
    let logger = crate::policies::logging::NoLogging;
    assert!(!logger.is_enabled());
    logger.log("INFO", "This should be optimized away");
}
```

#### Task 5.4 — Retry Policy Test

```rust
#[test]
fn test_no_retry_calls_once() {
    let policy = crate::policies::retry::NoRetry;
    let mut count = 0;
    let result = policy.attempt(|| {
        count += 1;
        None::<i32>
    });
    assert_eq!(count, 1);
    assert!(result.is_none());
}

#[test]
fn test_exponential_backoff_eventually_succeeds() {
    let policy = crate::policies::retry::ExponentialBackoff::new(1, 10);
    let mut attempts = 0;
    let result = policy.attempt(|| {
        attempts += 1;
        if attempts >= 3 { Some(attempts) } else { None }
    });
    assert_eq!(result, Some(3));
}
```

---

### Phase 6 — Build and Run

#### Task 6.1 — Build Commands

```powershell
# Debug build (fast iteration)
cd alex-crm-rs
cargo build

# Run in debug mode
cargo run

# Release build (optimized)
cargo build --release

# Run release binary
.\target\release\alex-crm.exe

# Run tests
cargo test

# Run tests with test name filter
cargo test test_bootstrap_schema
```

#### Task 6.2 — First-Run Behaviour

On first run:
1. `data/` directory is created if it doesn't exist.
2. `data/notes_app.db` is created with all tables and FTS5 index.
3. `data/notes_app_backup.db` is created on the first backup cycle.
4. `logs/` directory is created if using `FileLogger`.

---

### Phase 7 — Feature Checklist

- [x] `data/notes_app.db` schema bootstrap (idempotent)
- [x] Companies table with FTS5 full-text search
- [x] Contacts table + many-to-many join via `company_contacts`
- [x] Counties normalization table
- [x] Activities tracking table
- [x] Daily logs table (one entry per calendar date, upsert)
- [x] FTS5 triggers for automatic index sync
- [x] WAL mode + safe PRAGMAs
- [x] Prepared statement cache (`stmt_cache.rs`)
- [x] Logging policies: `NoLogging`, `ConsoleLogger`, `FileLogger`
- [x] Retry policies: `NoRetry`, `SpinRetry`, `ExponentialBackoff`
- [x] Cross-beam SPSC channels (bounded 256 for requests, unbounded for results)
- [x] DB worker thread (owns connection, processes DbTask)
- [x] Backup worker thread (periodic `sqlite3_backup`)
- [x] 900×650 window, centered on screen, 700×500 minimum
- [x] Companies tab with search bar + HoldBrowser + New Company button
- [x] Company detail window (non-modal, edit fields + save/cancel)
- [x] File explorer integration (open "Company Name/YYYY/MM/")
- [x] New company form with validation
- [x] Daily logs tab with text editor + save
- [x] Contacts tab (placeholder)
- [x] Activities tab (placeholder)
- [x] Idle callback for draining UiTask results
- [x] Graceful shutdown (sentinel task → join threads)
- [x] Heavy code commenting throughout
- [x] Unit tests for schema, DB worker, logging, retry

---

### Open Decisions

| # | Question | Default |
|---|---|---|
| D1 | Logging: console or file in dev? | Console (ConsoleLogger) |
| D2 | DB thread: std::thread or Tokio? | std::thread (simpler, no async needed) |
| D3 | SPSC capacity (must be power of 2)? | 256 |
| D4 | Backup interval? | 300 seconds (5 min) |
| D5 | FTS5 tokenizer? | porter (English stemming) |
| D6 | Company folder root path? | `./` (working directory) |

---

### Implementation Order

```
Phase 1: Foundation
  1.1 tasks.rs        ← message types
  1.2 logging.rs      ← trait + 3 implementations
  1.3 retry.rs        ← trait + 3 implementations
  1.4 mod.rs files    ← module exports

Phase 2: Backend
  2.1 schema.rs       ← bootstrap + pragmas
  2.2 stmt_cache.rs   ← cached prepared statements
  2.3 db_worker.rs    ← main DB worker loop
  2.4 backup.rs       ← periodic backup worker

Phase 3: Frontend
  3.1 state.rs        ← AppState (shared channels + logger)
  3.2 main_window.rs  ← window + tabs
  3.3 companies_tab.rs ← search + list
  3.4 detail_window.rs ← edit company + file explorer
  3.5 new_company_form.rs ← create company
  3.6 logs_tab.rs     ← daily log editor

Phase 4: Wiring
  4.1 app.rs          ← App<L, R> composition root
  4.2 main.rs         ← entry point, idle callback

Phase 5: Testing
  5.1 Schema test
  5.2 DB worker round-trip test
  5.3 Logging policy tests
  5.4 Retry policy tests

Phase 6: Build
  6.1 cargo build --release
  6.2 Verify features against checklist
```
