use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A thread-safe cancellation token that can be shared across threads
///
/// This allows encoding operations to be cancelled from another thread.
///
/// # Example
///
/// ```rust
/// use ravif::*;
/// use std::thread;
/// use std::time::Duration;
///
/// let token = CancellationToken::new();
/// let token_clone = token.clone();
///
/// // Spawn a thread to cancel after 100ms
/// thread::spawn(move || {
///     thread::sleep(Duration::from_millis(100));
///     token_clone.cancel();
/// });
///
/// // This will be cancelled if encoding takes > 100ms
/// let encoder = Encoder::new()
///     .with_quality(70.0)
///     .with_cancellation_token(token);
///
/// // encode_rgba() will return Error::Cancelled if cancelled
/// ```
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new cancellation token
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the operation
    ///
    /// This sets the cancellation flag. Any encoding operations using this token
    /// will check the flag periodically and return `Error::Cancelled`.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Check if cancellation has been requested
    ///
    /// Returns `true` if `cancel()` has been called.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Reset the cancellation state
    ///
    /// This allows reusing the same token for multiple operations.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        token.cancel();
        assert!(token.is_cancelled());

        token.reset();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancellation_token_clone() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(!token.is_cancelled());
        assert!(!clone.is_cancelled());

        clone.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }
}
