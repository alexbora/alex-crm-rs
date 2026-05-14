# Alex CRM — Detailed Development Plan

> **Stack:** C++26 · FLTK (GUI thread) · SQLite3 FTS5 (DB worker thread)  
> **Build:** Ninja + CMake presets · Windows/MSYS2 UCRT64 · static linking  
> **Architecture:** `App<LoggingPolicy, RetryPolicy>` template; SPSC atomic queue per side; no
> shared state between threads except the queues.

---

## 0 — Glossary / Architecture Summary

```
main()
  └─ App<ConsoleLogger, ExponentialBackoff>
       ├─ Frontend           (FLTK event loop, UI thread)
       │    └─ SpscQueue<UiTask>   ← backend posts results here
       └─ RuntimeServices   (DB worker + backup worker threads)
            └─ SpscQueue<DbTask>   ← frontend posts requests here
```

Each side only touches its *own* output queue and the other side's *input* queue
via a single atomic write/read pair.  No mutex crosses the boundary.

---

## Phase 1 — Foundation & Infrastructure

### Task 1.1 — SPSC Atomic Queue (`include/SpscQueue.hpp`)

**Goal:** A lock-free, cache-line-padded SPSC ring buffer template.

**Details:**
- Template parameter: `T` (task type), `Capacity` (power of 2, default 256).
- `head_` and `tail_` each sit on their own 64-byte aligned cache line (prevent
  false sharing).
- `try_push(T&&)` → `bool`; `try_pop(T&)` → `bool`; both are `noexcept`.
- Use `std::atomic<std::size_t>` with `memory_order_acquire` / `memory_order_release`.
- Static assertion: `Capacity` is a power of 2.
- Header-only; no dependencies beyond `<atomic>` and `<array>`.

**Acceptance:** Unit test in `TESTS/test_spsc.cpp` that pushes 1 M items from one
thread and pops from another, verifying order and no item loss.

---

### Task 1.2 — Logging Policy (`Logging.hpp`)

**Goal:** Three compile-time logging policies selected via template argument.

**Policies:**
- `NoLogging` — all methods are empty and `[[maybe_unused]]`; zero overhead.
- `ConsoleLogger` — writes to `std::cerr` with a timestamp prefix.
- `FileLogger` — writes to a rotating daily log file under `logs/YYYY-MM-DD.log`
  (creates the directory if absent).

**Interface (each policy must satisfy):**
```cpp
void log(std::string_view level, std::string_view message);
void flush();           // no-op for console/no-op; fflush for file
bool is_enabled() const noexcept;
```

**Notes:**
- `FileLogger` must be move-constructible (for `App` ownership).
- Thread-safe: each policy must serialize concurrent `log()` calls with a
  `std::mutex` (file/console) or be truly empty (none).
- Comment every method explaining the design choice.

---

### Task 1.3 — Retry / Backoff Policy (`include/RetryPolicy.hpp`)

**Goal:** Compile-time retry strategies for queue push operations.

**Policies:**
- `NoRetry` — try once, return result immediately.
- `SpinRetry<N>` — busy-spin up to N times, then yield.
- `ExponentialBackoff<MinNs, MaxNs>` — sleep doubles from MinNs to MaxNs, then
  caps; uses `std::this_thread::sleep_for`.

**Interface:**
```cpp
template <typename Fn>   // Fn() → bool
bool attempt(Fn&& fn);   // returns true if fn() eventually returned true
```

---

### Task 1.4 — Task Type Definitions (`include/Tasks.hpp`)

**Goal:** Plain value types for every cross-thread message.

**UI-bound results (backend → frontend):**
```cpp
struct FetchCompaniesResult { std::vector<CompanyRow> rows; std::uint64_t generation; };
struct InsertCompanyResult  { bool success; std::string error_message; };
struct UpdateCompanyResult  { bool success; std::string error_message; };
struct DeleteCompanyResult  { bool success; std::string error_message; };
struct BackupStatusResult   { std::string message; };
```

**DB-bound requests (frontend → backend):**
```cpp
struct FetchCompaniesReq    { std::string search; std::size_t offset; std::size_t limit; std::uint64_t generation; };
struct InsertCompanyReq     { std::string name; std::string county; std::string contact_first; std::string contact_last; };
struct UpdateCompanyReq     { std::int64_t id; std::string new_name; };
struct DeleteCompanyReq     { std::int64_t id; };
struct RequestBackupReq     {};
```

Each request carries a `std::function<void(Result)>` callback stored inline
(std::variant or a tagged union keeps the queue element size fixed).

---

## Phase 2 — Backend / Database Layer

### Task 2.1 — Database Schema & Migration (`backend/schema.hpp`)

**Goal:** Idempotent schema bootstrap in a single function called at startup.

**Tables to create (IF NOT EXISTS):**
```sql
companies      (id INTEGER PK, name TEXT UNIQUE NOT NULL, county_id INT FK)
contacts       (id INTEGER PK, name TEXT NOT NULL, last_name TEXT)
company_contacts (company_id FK, contact_id FK, PRIMARY KEY both)
counties       (id INTEGER PK, name TEXT UNIQUE NOT NULL)
activities     (id INTEGER PK, company_id FK, type TEXT, description TEXT, created_at TEXT)
daily_logs     (id INTEGER PK, log_date TEXT, entry TEXT, created_at TEXT)
companies_fts  (FTS5 virtual table, tokenize='camel', content='companies')
```

**Triggers:**
- `companies_ai/ad/au` — keep `companies_fts` in sync with `companies`.

**PRAGMAs applied at every connection open:**
```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous   = NORMAL;
PRAGMA busy_timeout  = 5000;
```

**Acceptance:** Schema function is called with a fresh in-memory DB and all
`SELECT` queries against `sqlite_master` return the expected objects.

---

### Task 2.2 — Statement Cache (`backend/StmtCache.hpp`)

**Goal:** Per-connection lazy-prepared statement cache; `finalize_all()` called
before `sqlite3_close`.

**Cached statements (one `sqlite3_stmt*` each):**
| Name | SQL |
|---|---|
| `fetch_all` | `SELECT id, name FROM companies ORDER BY name COLLATE NOCASE LIMIT ? OFFSET ?` |
| `fetch_fts` | `SELECT c.id, c.name FROM companies c JOIN companies_fts f ON c.id=f.rowid WHERE companies_fts MATCH ? ORDER BY rank LIMIT ? OFFSET ?` |
| `count_all` | `SELECT COUNT(*) FROM companies` |
| `count_fts` | `SELECT COUNT(*) FROM companies_fts WHERE companies_fts MATCH ?` |
| `insert_company` | `INSERT INTO companies(name) VALUES(?)` |
| `insert_company_full` | same + bind county + insert/link contacts in TX |
| `update_company` | `UPDATE companies SET name=? WHERE id=?` |
| `delete_company` | `DELETE FROM companies WHERE id=?` |
| `begin_immediate` | `BEGIN IMMEDIATE` |
| `commit` | `COMMIT` |
| `rollback` | `ROLLBACK` |
| `get_county` | `SELECT id FROM counties WHERE name=? COLLATE NOCASE` |
| `insert_county` | `INSERT OR IGNORE INTO counties(name) VALUES(?)` |
| `get_contact` | `SELECT id FROM contacts WHERE name=? AND last_name=?` |
| `insert_contact` | `INSERT INTO contacts(name, last_name) VALUES(?,?)` |
| `link_contact` | `INSERT OR IGNORE INTO company_contacts VALUES(?,?)` |
| `insert_activity` | `INSERT INTO activities(company_id, type, description, created_at) VALUES(?,?,?,datetime('now'))` |
| `insert_daily_log` | `INSERT INTO daily_logs(log_date, entry, created_at) VALUES(?,?,datetime('now'))` |

Statements are prepared on first use; `reset()` + `clear_bindings()` after every
execution so they stay reusable.

---

### Task 2.3 — DB Worker Thread (`backend/backend.cpp`)

**Goal:** Single long-lived thread that owns the SQLite connection and the
`StmtCache`.

**Behaviour:**
- Pops `DbTask` from its `SpscQueue` using `ExponentialBackoff`.
- Dispatches to the correct handler (fetch / insert / update / delete / backup).
- On insert/update/delete: wraps in `BEGIN IMMEDIATE … COMMIT`; on constraint
  error builds a friendly message and posts failure result back to UI.
- On fetch: decides between plain `SELECT` and FTS query based on whether
  `search` is non-empty.
- After each successful write: posts a `FetchCompaniesResult` back to UI so the
  list refreshes automatically.
- Backup handler: uses `sqlite3_backup_*` API into the backup file; runs
  `PRAGMA optimize` first.
- Thread exits cleanly when a sentinel task (`std::monostate`) is received.

---

### Task 2.4 — Backup Worker Thread (`backend/backend.cpp`)

**Goal:** Second background thread — does not touch the main SQLite connection.

**Behaviour:**
- Waits on a `std::condition_variable` (no busy-spin needed; backup is rare).
- Wakes on: `request_backup_now()` call or when `backup_interval_` elapses.
- Opens source DB read-only, opens/creates backup DB read-write.
- Steps backup 500 pages at a time; sleeps 25 ms on `SQLITE_BUSY/LOCKED`.
- Notifies UI via `SpscQueue` on start / completion / failure.

---

## Phase 3 — Frontend / UI Layer

### Task 3.1 — App Template (`app.hpp`)

**Goal:** Composition root that owns both sides and wires their queues together.

```cpp
template <typename LoggingPolicy, typename RetryPolicy>
class App {
    SpscQueue<DbTask>  db_queue_;   // frontend pushes here
    SpscQueue<UiTask>  ui_queue_;   // backend pushes here
    Frontend           frontend_;
    RuntimeServices    backend_;
public:
    int run(int argc, char** argv);
};
```

- `run()` starts the backend threads, registers an `Fl::add_idle` callback that
  drains `ui_queue_` on each FLTK idle tick, then calls `Fl::run()`.
- On window close: posts sentinel to backend, joins threads, then returns.
- Logging policy is stored `inline static std::optional<LoggingPolicy>` and
  accessible via `App::log(level, msg)` from any callback.

---

### Task 3.2 — Main Window Layout (`frontend/frontend.cpp`)

**Goal:** Build the FLTK window with four tabs, centered on screen at startup.

**Window:**
- Title: `"Alex CRM"`
- Size: 900 × 650 (resizable, minimum 700 × 500).
- Centered: `x = (Fl::w() - w) / 2; y = (Fl::h() - h) / 2`.
- Font: load `Hermit-Regular.otf` via `Fl::set_font` if found, fall back to
  system `FL_HELVETICA`.

**Tabs:**
| Index | Label | Content |
|---|---|---|
| 0 | `Companies` | company browser + search bar |
| 1 | `Contacts` | placeholder `Fl_Box` "Coming soon" |
| 2 | `Activities` | placeholder `Fl_Box` "Coming soon" |
| 3 | `Logs` | daily log entry form (see Task 3.6) |

---

### Task 3.3 — Companies Tab (`frontend/frontend.cpp`)

**Goal:** Scrollable, searchable list of companies backed by the virtual-list
pattern already in the codebase.

**Sub-components:**
- `Fl_Input` search bar (top, full width minus padding).
- `CrmHoldBrowser` company list (fills remaining height).
- `Fl_Button` "New Company" (bottom-right corner).

**Behaviour:**
- On tab shown / startup: post `FetchCompaniesReq{search="", offset=0, limit=500}`.
- On search input change (debounced 150 ms via `Fl::add_timeout`): post new
  fetch with current search text.
- On list item single-click: do nothing (highlight only).
- On list item double-click / `ENTER`: open Company Detail Window (Task 3.4).
- Right-click context menu: `Edit`, `Delete`, `Open Folder`.
  - `Delete` shows `fl_ask()` confirmation dialog before posting `DeleteCompanyReq`.
- List refresh: receives `FetchCompaniesResult` from UI queue; calls
  `VirtualCompanyList::set_rows(rows)` which redraws.

---

### Task 3.4 — Company Detail Window (`frontend/frontend.cpp`)

**Goal:** Secondary `Fl_Window` opened non-modally when a company is activated.

**Fields displayed / editable:**
- Company name (`Fl_Input`, full width).
- County (`Fl_Input` or `Fl_Choice` if counties already exist in DB).
- Primary contact first + last name (`Fl_Input` pair).

**Buttons:**
- `Save` — posts `UpdateCompanyReq`; closes window on success; shows
  `fl_alert()` on error.
- `Cancel` — closes window without changes.

**Side effect on open:**
- Call `ShellExecuteW(NULL, L"explore", path, ...)` (Windows) where
  `path = "Company name\\YYYY\\MM"` under the working directory.
  Create the directory if it does not yet exist using `std::filesystem::create_directories`.
- Comment the Windows-only `#ifdef _WIN32` guard.

---

### Task 3.5 — New Company Form (`frontend/frontend.cpp`)

**Goal:** Secondary `Fl_Window` opened from the "New Company" button.

**Fields:**
- Company name (required).
- County (optional).
- Contact first name (optional).
- Contact last name (optional).

**Validation:** Name must be non-empty and not purely whitespace. Show inline
`Fl_Box` error message (red text) rather than a dialog.

**On Save:** post `InsertCompanyReq`; on success close form; on duplicate name
show `"Company '<name>' already exists."` in the error box.

---

### Task 3.6 — Logs Tab (`frontend/frontend.cpp`)

**Goal:** Simple daily log editor.

**Layout:**
- Top: `Fl_Box` showing today's date (auto-updated at midnight via timer).
- Middle: `Fl_Text_Editor` for multi-line text entry.
- Bottom: `Save Entry` button.

**Behaviour:**
- On tab shown: post `FetchDailyLogReq{date=today}` to load existing entry for
  today if any.
- On save: post `InsertDailyLogReq{date, text}`.

---

## Phase 4 — Policies & App Wiring

### Task 4.1 — Finalize `main.cpp`

**Goal:** Wire everything together and resolve DB path.

```cpp
int main(int argc, char** argv) {
    using MyApp = App<ConsoleLogger, ExponentialBackoff<1000, 64000>>;
    MyApp::configure_logger();
    auto source = find_existing_path({...});
    auto backup = source.parent_path() / "notes_app_backup.db";
    MyApp app;
    return app.run(argc, argv, source.string(), backup.string(),
                   std::chrono::minutes(5));
}
```

---

### Task 4.2 — CMake Wiring

**Goal:** Ensure all new translation units are listed in `CMakeLists.txt`.

New files to add to the `app` and `test_backend` targets:
- `backend/schema.cpp` (if schema bootstrap is split out)
- Any new `src/*.cpp` helper

Confirm `include/` directory is in `target_include_directories`.

---

## Phase 5 — Testing

### Task 5.1 — SPSC Queue Unit Test (`TESTS/test_spsc.cpp`)

- Producer thread pushes integers 0 … 999 999.
- Consumer thread pops and accumulates sum.
- Assert sum equals expected.
- Run under TSan if available.

### Task 5.2 — Schema Bootstrap Test (`TESTS/test.cpp`)

- Open in-memory DB, call `bootstrap_schema(db)`.
- Query `sqlite_master` for each expected table, trigger, and virtual table.
- Assert all present.

### Task 5.3 — Statement Cache Test (`TESTS/test.cpp`)

- Open temp file DB, bootstrap schema, create `StmtCache`.
- Insert 5 companies, fetch all, verify count and names.
- Insert duplicate, verify friendly error message.
- Delete one, verify count drops to 4.

### Task 5.4 — Backend Round-Trip Test (`TESTS/test.cpp`)

- Instantiate `RuntimeServicesRefactored` with a temp DB.
- Call `insert_company_sync("Acme")`, assert success.
- Call `fetch_company_window_sync({})`, assert Acme is in results.
- Call `update_company_sync(id, "Acme Corp")`, assert success.
- Call `delete_company_sync(id)`, assert success.

---

## Phase 6 — Documentation & Polish

### Task 6.1 — Code Comments

Per spec: **comment heavily**.

- Every class: one block comment explaining its role in the architecture.
- Every public method: doc comment with `// Purpose:`, `// Thread:`, `// Params:`, `// Returns:`.
- Every FLTK callback: explain what UI action triggers it and what it posts.
- Every SPSC queue interaction: note which thread is producer and which is consumer.

### Task 6.2 — README update

Update `README.md` / `AGENTS.md` to reflect:
- Final file layout.
- Build commands (presets).
- Run commands.
- New policy template parameters.

### Task 6.3 — `Insertion.mk` hygiene

Verify `Insertion.mk` still builds `insert_companies.exe` cleanly with the
current schema (companies + counties + contacts tables exist).

---

## Implementation Order

```
1.1 SpscQueue          ← everything depends on the queue
1.2 Logging.hpp        ← used by App and backend
1.3 RetryPolicy.hpp    ← used by App
1.4 Tasks.hpp          ← cross-thread message types
2.1 schema.hpp/cpp     ← DB layer foundation
2.2 StmtCache.hpp      ← built on schema
2.3 DB worker          ← uses queue + cache
2.4 Backup worker      ← independent of DB worker
3.1 App template       ← wires queues
3.2 Main window        ← FLTK shell
3.3 Companies tab      ← primary data surface
3.4 Detail window      ← edit flow
3.5 New company form   ← insert flow
3.6 Logs tab           ← secondary feature
4.1 main.cpp           ← final wiring
4.2 CMake              ← ensure targets build
5.x Tests              ← validate each layer
6.x Docs               ← finalize
```

---

## Open Decisions (confirm before implementing)

| # | Question | Default assumption |
|---|---|---|
| D1 | Should `counties` be a free-text input or a `Fl_Choice` populated from DB? | Free text input; normalize to `counties` table on save |
| D2 | Company folder root: current working directory or a user-configurable path? | `./` (next to the exe) |
| D3 | SPSC queue capacity (must be power of 2)? | 256 per side |
| D4 | Backup interval default? | 5 minutes (already in `main.cpp`) |
| D5 | `Logs` tab: one entry per day (overwrite) or append multiple entries? | One editable entry per calendar day |
| D6 | Activities tab: deferred to a future milestone or scaffold now? | Scaffold placeholder only |
