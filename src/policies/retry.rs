use std::thread;
use std::time::Duration;

pub trait RetryPolicy: Send + Sync {
    fn attempt<T, F>(&self, operation: F) -> Result<T, String>
    where
        F: FnMut() -> Result<T, String>;
}

#[allow(dead_code)]
pub struct NoRetry;

impl RetryPolicy for NoRetry {
    fn attempt<T, F>(&self, mut operation: F) -> Result<T, String>
    where
        F: FnMut() -> Result<T, String>,
    {
        operation()
    }
}

#[allow(dead_code)]
pub struct SpinRetry {
    attempts: usize,
}

impl SpinRetry {
    #[allow(dead_code)]
    pub fn new(attempts: usize) -> Self {
        Self {
            attempts: attempts.max(1),
        }
    }
}

impl RetryPolicy for SpinRetry {
    fn attempt<T, F>(&self, mut operation: F) -> Result<T, String>
    where
        F: FnMut() -> Result<T, String>,
    {
        let mut last_error = String::from("operation failed");
        for _ in 0..self.attempts {
            match operation() {
                Ok(value) => return Ok(value),
                Err(err) => last_error = err,
            }
        }
        Err(last_error)
    }
}

pub struct ExponentialBackoff {
    min_ms: u64,
    max_ms: u64,
    max_attempts: usize,
}

impl ExponentialBackoff {
    pub fn new(min_ms: u64, max_ms: u64) -> Self {
        Self {
            min_ms: min_ms.max(1),
            max_ms: max_ms.max(min_ms.max(1)),
            max_attempts: 8,
        }
    }

    #[allow(dead_code)]
    pub fn with_attempts(min_ms: u64, max_ms: u64, max_attempts: usize) -> Self {
        Self {
            min_ms: min_ms.max(1),
            max_ms: max_ms.max(min_ms.max(1)),
            max_attempts: max_attempts.max(1),
        }
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn attempt<T, F>(&self, mut operation: F) -> Result<T, String>
    where
        F: FnMut() -> Result<T, String>,
    {
        let mut delay_ms = self.min_ms;
        let mut last_error = String::from("operation failed");

        for attempt in 0..self.max_attempts {
            match operation() {
                Ok(value) => return Ok(value),
                Err(err) => {
                    last_error = err;
                    if attempt + 1 < self.max_attempts {
                        thread::sleep(Duration::from_millis(delay_ms));
                        delay_ms = (delay_ms.saturating_mul(2)).min(self.max_ms);
                    }
                }
            }
        }

        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExponentialBackoff, NoRetry, RetryPolicy, SpinRetry};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn no_retry_calls_once() {
        let retry = NoRetry;
        let calls = AtomicUsize::new(0);
        let result = retry.attempt(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, String>(42)
        });
        assert_eq!(result.unwrap_or_default(), 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn spin_retry_retries() {
        let retry = SpinRetry::new(3);
        let calls = AtomicUsize::new(0);
        let result = retry.attempt(|| {
            let attempt = calls.fetch_add(1, Ordering::Relaxed) + 1;
            if attempt < 3 {
                Err("busy".to_string())
            } else {
                Ok(7)
            }
        });
        assert_eq!(result.unwrap_or_default(), 7);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn exponential_backoff_stops_after_limit() {
        let retry = ExponentialBackoff::with_attempts(1, 2, 2);
        let result = retry.attempt::<(), _>(|| Err("fail".to_string()));
        assert!(result.is_err());
    }
}
