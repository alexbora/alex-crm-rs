use chrono::Local;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub trait LoggingPolicy: Send + Sync {
    fn log(&self, level: &str, message: &str);
    fn flush(&self);
    #[allow(dead_code)]
    fn is_enabled(&self) -> bool;
}

#[allow(dead_code)]
pub struct NoLogging;

impl LoggingPolicy for NoLogging {
    fn log(&self, _level: &str, _message: &str) {}
    fn flush(&self) {}
    fn is_enabled(&self) -> bool {
        false
    }
}

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
        if self.enabled {
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("[{}] {}: {}", now, level, message);
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[allow(dead_code)]
pub struct FileLogger {
    log_dir: PathBuf,
    lock: Mutex<()>,
}

#[allow(dead_code)]
impl FileLogger {
    #[allow(dead_code)]
    pub fn new(log_dir: impl Into<PathBuf>) -> Self {
        let log_dir = log_dir.into();
        let _ = create_dir_all(&log_dir);
        Self {
            log_dir,
            lock: Mutex::new(()),
        }
    }

    #[allow(dead_code)]
    fn daily_log_path(&self) -> PathBuf {
        let date = Local::now().format("%Y-%m-%d").to_string();
        self.log_dir.join(format!("{date}.log"))
    }
}

impl LoggingPolicy for FileLogger {
    fn log(&self, level: &str, message: &str) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };

        let path = self.daily_log_path();
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let now = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{}] {}: {}", now, level, message);
        }
    }

    fn flush(&self) {}

    fn is_enabled(&self) -> bool {
        true
    }
}
