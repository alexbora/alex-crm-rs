mod app;
mod backend;
mod policies;
mod state;
mod tasks;
mod ui;

use app::App;
use backend::backup_worker::{BackupCommand, BackupWorker};
use backend::db_worker::DbWorker;
use crossbeam::channel;
use policies::{ConsoleLogger, ExponentialBackoff, LoggingPolicy};
use state::AppState;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tasks::{DbTask, UiTask};

fn main() {
    let logger = Arc::new(ConsoleLogger::new(true));
    let retry_policy = Arc::new(ExponentialBackoff::new(2, 128));
    logger.log("INFO", "Starting Alex CRM");

    let (db_tx, db_rx) = channel::bounded::<DbTask>(256);
    let (ui_tx, ui_rx) = channel::bounded::<UiTask>(256);
    let (backup_tx, backup_rx) = channel::bounded::<BackupCommand>(32);

    let (source_db, backup_db) = resolve_db_paths();
    logger.log(
        "INFO",
        &format!("Using database: {}", source_db.to_string_lossy()),
    );
    let backup_source_db = source_db.clone();
    let db_source_db = source_db;

    let backup_logger = logger.clone();
    let backup_retry = retry_policy.clone();
    let backup_ui_tx = ui_tx.clone();
    let backup_thread = thread::spawn(move || {
        let worker = BackupWorker::new(
            backup_source_db,
            backup_db,
            Duration::from_secs(300),
            backup_logger,
            backup_retry,
        );
        worker.run(backup_rx, backup_ui_tx);
    });

    let db_logger = logger.clone();
    let db_retry = retry_policy.clone();
    let db_ui_tx = ui_tx.clone();
    let db_backup_tx = backup_tx.clone();
    let db_thread = thread::spawn(move || {
        let worker = DbWorker::new(db_source_db, db_logger, db_retry, db_backup_tx);
        worker.run(db_rx, db_ui_tx);
    });

    let app_state = AppState::new(db_tx.clone(), ui_rx);
    let app = App::new(logger.clone(), retry_policy.clone(), app_state);
    app.run();

    let _ = db_tx.send(DbTask::Shutdown);
    let _ = backup_tx.send(BackupCommand::Shutdown);
    let _ = db_thread.join();
    let _ = backup_thread.join();

    logger.log("INFO", "Alex CRM closed");
    logger.flush();
}

fn resolve_db_paths() -> (PathBuf, PathBuf) {
    if let Some(arg_db) = env::args().nth(1) {
        let source_db = PathBuf::from(arg_db);
        if let Some(parent) = source_db.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let backup_db = derive_backup_path(&source_db);
        return (source_db, backup_db);
    }

    let data_dir = PathBuf::from(r"E:\alex-crm-rs\data");
    let _ = std::fs::create_dir_all(&data_dir);
    (
        data_dir.join("notes_app.db"),
        data_dir.join("notes_app_backup.db"),
    )
}

fn derive_backup_path(source_db: &PathBuf) -> PathBuf {
    let parent = source_db
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = source_db
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("notes_app");
    let ext = source_db
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("db");

    parent.join(format!("{stem}_backup.{ext}"))
}
