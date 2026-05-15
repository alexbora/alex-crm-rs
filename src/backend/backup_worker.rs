use crate::policies::{LoggingPolicy, RetryPolicy};
use crate::tasks::{OperationResult, UiTask};
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum BackupCommand {
    RunNow,
    Shutdown,
}

pub struct BackupWorker<L: LoggingPolicy, R: RetryPolicy> {
    source_db: PathBuf,
    backup_db: PathBuf,
    interval: Duration,
    logger: Arc<L>,
    retry_policy: Arc<R>,
}

impl<L: LoggingPolicy, R: RetryPolicy> BackupWorker<L, R> {
    pub fn new(
        source_db: PathBuf,
        backup_db: PathBuf,
        interval: Duration,
        logger: Arc<L>,
        retry_policy: Arc<R>,
    ) -> Self {
        Self {
            source_db,
            backup_db,
            interval,
            logger,
            retry_policy,
        }
    }

    pub fn run(&self, rx: Receiver<BackupCommand>, ui_tx: Sender<UiTask>) {
        self.logger.log("INFO", "Backup worker started");

        loop {
            match rx.recv_timeout(self.interval) {
                Ok(BackupCommand::RunNow) => {
                    self.perform_with_reporting(&ui_tx, "Manual backup completed.");
                }
                Ok(BackupCommand::Shutdown) => {
                    self.logger.log("INFO", "Backup worker shutting down");
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.perform_with_reporting(&ui_tx, "Scheduled backup completed.");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.logger.log("WARN", "Backup command channel disconnected");
                    break;
                }
            }
        }
    }

    fn perform_with_reporting(&self, ui_tx: &Sender<UiTask>, success_message: &str) {
        let source = self.source_db.clone();
        let backup = self.backup_db.clone();

        let result = self
            .retry_policy
            .attempt(|| perform_backup_copy(source.clone(), backup.clone()))
            .map(|_| OperationResult {
                success: true,
                message: success_message.to_string(),
            })
            .map_err(|err| format!("Backup failed: {err}"));

        match &result {
            Ok(ok) => self.logger.log("INFO", &ok.message),
            Err(err) => self.logger.log("ERROR", err),
        }

        let _ = ui_tx.send(UiTask::BackupStatusResult(result));
    }
}

fn perform_backup_copy(source_db: PathBuf, backup_db: PathBuf) -> Result<(), String> {
    if !source_db.exists() {
        return Err(format!(
            "Source database does not exist yet: {}",
            source_db.display()
        ));
    }

    if let Some(parent) = backup_db.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let tmp_path = backup_db.with_extension("tmp");
    fs::copy(&source_db, &tmp_path).map_err(|e| e.to_string())?;
    if backup_db.exists() {
        fs::remove_file(&backup_db).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp_path, &backup_db).map_err(|e| e.to_string())?;
    Ok(())
}
